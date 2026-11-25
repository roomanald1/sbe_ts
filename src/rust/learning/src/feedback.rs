use super::*;

pub(super) const MINIMUM_SCOUT_PARAMETER_SAMPLES: usize = 10;
pub(super) const MAXIMUM_SCOUT_PARAMETER_SAMPLES: usize = 100;
pub(super) const PARAMETER_STEP_RELATIVE_TOLERANCE: f64 = 1.0e-6;

pub(super) async fn accept_feedback(
    State(service): State<ServiceState>,
    Json(event): Json<FeedbackEvent>,
) -> Result<Json<FeedbackResult>, (StatusCode, String)> {
    let (_, symbol, _, _, _) = event_parts(&event);
    if !strategy::is_autotrader_symbol_allowed(symbol) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "symbol is outside the AutoTrader allowlist".into(),
        ));
    }
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
        let invalid_field = if event_id.trim().is_empty() || event_id.len() > 256 {
            Some("event_id")
        } else if symbol.trim().is_empty() || symbol.len() > 64 {
            Some("symbol")
        } else if strategy.is_empty() || strategy.len() > 64 || !strategy.starts_with("live-") {
            Some("strategy")
        } else if profile_id.trim().is_empty() || profile_id.len() > 128 {
            Some("profile_id")
        } else if parameters.len() > 64 {
            Some("parameters.count")
        } else if parameters.keys().any(|key| key.len() > 128) {
            Some("parameters.key_length")
        } else if parameters.values().any(|value| !value.is_finite()) {
            Some("parameters.value")
        } else if !matches!(direction.as_str(), "bullish" | "bearish") {
            Some("direction")
        } else if !strength.is_finite() {
            Some("strength")
        } else if !score.is_finite() {
            Some("score")
        } else if learning_context.len() > 256 {
            Some("learning_context")
        } else {
            None
        };
        if let Some(invalid_field) = invalid_field {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("invalid missed-opportunity candidate: {invalid_field}"),
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
        // Keep the in-memory idempotency state aligned with the durable
        // commit. If SQLite rejects this write, a retry in this process must
        // be able to apply the event again.
        let before_feedback = state.clone();
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
        if let Err(error) = persist(&service.path, &state) {
            *state = before_feedback;
            return Err(error);
        }
        return Ok(Json(FeedbackResult {
            accepted: true,
            duplicate: false,
            profile: None,
        }));
    }
    let (event_id, symbol, strategy, parameters, profile_id) = event_parts(&event);
    validate_feedback_identity(event_id, symbol, strategy, &profile_id, parameters)?;
    if matches!(
        &event,
        FeedbackEvent::ScoutOutcome {
            market_regime: Some(regime),
            ..
        } if !matches!(regime.as_str(), "range" | "trend" | "volatile" | "mixed")
    ) {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid Scout market regime".into(),
        ));
    }
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

    // Mutations below update profiles, replay jobs and the in-memory dedupe
    // set together. Roll them all back if the corresponding SQLite
    // transaction fails, so a same-process retry cannot be mistaken for a
    // durable duplicate.
    let before_feedback = state.clone();
    let evidence_gain_now_ms = now_ms();
    let evidence_gain_cohort_id = evidence_gain_scope(&state, &event).0;
    let evidence_gain_before = evidence_gain_coverage_snapshot(
        &state,
        evidence_gain_cohort_id.as_deref(),
        evidence_gain_now_ms,
    );
    let profile_key = key(symbol, strategy);
    retention::ensure_active_profile_room(&state, &profile_key)?;
    if let Some(context) = event_context(&event) {
        retention::ensure_context_room(&state, &context)?;
    }
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
    let full_persist_required = matches!(&event, FeedbackEvent::ScoutOutcome { .. })
        && !state.variants.contains_key(&variant_key(&source_profile))
        && state.variants.len() >= retention::MAX_VARIANTS_IN_MEMORY;
    register_feedback_variant(&mut state, &event, source_profile, parameters, &profile_id)?;
    register_scout_baseline_if_missing(&mut state, &event, symbol, strategy);
    let (update, outcome_history_compacted) =
        append_profile_outcome(&mut state, &event, symbol, strategy, parameters)?;
    update_active_profile(&mut state, &event, &profile_key, &update);
    if matches!(&event, FeedbackEvent::ScoutOutcome { .. }) {
        let minimum_parameter_samples = state.minimum_live_scout_parameter_samples;
        let candidate_family_id = event_candidate_family_id(&event);
        let triggering_block_id = event_independence_block_id(&event);
        promote_qualified_scout_candidates(
        &mut state,
        symbol,
        strategy,
        minimum_parameter_samples,
            Some(event_id),
            candidate_family_id.as_deref(),
            triggering_block_id,
    );
    }
    train_threshold_parameters(&mut state, &event, symbol, strategy, &profile_key, &update);
    enqueue_strategy_replay(&mut state, &event);
    finalize_feedback(&mut state, &event, event_id);
    let evidence_gain_changed = record_evidence_gain_event(
        &mut state,
        &event,
        evidence_gain_before,
        evidence_gain_now_ms,
    );
    let persistence = if matches!(&event, FeedbackEvent::ScoutOutcome { .. })
        && !full_persist_required
        && !outcome_history_compacted
        && !evidence_gain_changed
    {
        super::persist_scout_outcome_delta(&service.path, &state, &event)
    } else {
        persist(&service.path, &state)
    };
    if let Err(error) = persistence {
        *state = before_feedback;
        return Err(error);
    }
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
        replay_context,
        scout,
        schema_version,
        event_id,
        episode_id,
        opportunity_id,
        trade_id,
        profile_hash,
        profile_version,
        execution_mode,
        entry_policy,
        source,
        ..
    } = event
    else {
        return Ok(());
    };
    // WP14 outcomes are identified by their entry policy. Some historical
    // producers set `scout=false` for exploratory trades, so the scout flag
    // alone is not an authoritative lineage gate.
    if (*scout || entry_policy.as_deref() == Some("exploratory"))
        && schema_version.is_some_and(|version| version >= 1)
    {
        let lineage = replay_context
            .as_ref()
            .and_then(|context| context.lineage.as_ref());
        let complete = lineage.is_some_and(|lineage| {
            let nonempty = |value: &Option<String>| {
                value.as_deref().is_some_and(|text| !text.trim().is_empty())
            };
            lineage.outcome_event_id == *event_id
                && nonempty(&lineage.source_event_id)
                && lineage.source_event_id != lineage.episode_id
                && nonempty(&lineage.episode_id)
                && lineage.episode_id.as_deref() != Some(lineage.outcome_event_id.as_str())
                && lineage.episode_id == *episode_id
                && nonempty(&lineage.opportunity_id)
                && lineage.opportunity_id == *opportunity_id
                && nonempty(&lineage.trade_id)
                && lineage.trade_id == *trade_id
                && nonempty(&lineage.profile_hash)
                && lineage.profile_hash == *profile_hash
                && lineage.profile_version.is_some()
                && lineage.profile_version == *profile_version
                && nonempty(&lineage.execution_mode)
                && lineage.execution_mode == *execution_mode
                && nonempty(&lineage.entry_policy)
                && lineage.entry_policy == *entry_policy
                && nonempty(&lineage.source)
                && lineage.source == *source
                && !lineage.entry_fill_ids.is_empty()
                && !lineage.exit_fill_ids.is_empty()
                && lineage
                    .entry_fill_ids
                    .iter()
                    .chain(&lineage.exit_fill_ids)
                    .all(|id| !id.trim().is_empty())
                && lineage
                    .entry_fill_ids
                    .iter()
                    .all(|id| !lineage.exit_fill_ids.contains(id))
                && nonempty(&lineage.fee_currency)
                && lineage.fees_complete
        });
        if !complete {
            return Err((StatusCode::UNPROCESSABLE_ENTITY, "exploratory trade outcome lacks authoritative event, episode, reservation, fill, profile, or fee lineage".into()));
        }
    }
    let Some(context) = replay_context else {
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
        schema_version,
        trade_id,
        opportunity_id,
        episode_id,
        entry_profile_id,
        profile_hash,
        profile_version,
        execution_mode,
        entry_policy,
        source,
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
            schema_version: *schema_version,
            trade_id: trade_id.clone(),
            opportunity_id: opportunity_id.clone(),
            episode_id: episode_id.clone(),
            entry_profile_id: entry_profile_id.clone(),
            profile_hash: profile_hash.clone(),
            profile_version: *profile_version,
            execution_mode: execution_mode.clone(),
            entry_policy: entry_policy.clone(),
            source: source.clone(),
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
            history_status: "pending".into(),
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
    triggering_event_id: Option<&str>,
    triggering_family_id: Option<&str>,
    triggering_block_id: Option<u64>,
) {
    let mut candidates = BTreeMap::new();
    let mut family_hypotheses = BTreeMap::<String, BTreeSet<String>>::new();
    let mut family_sizes = BTreeMap::<String, Option<usize>>::new();
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
                && outcome.candidate_family_id.is_some()
                && outcome.candidate_family_size.is_some()
                && outcome.independence_block_id.is_some()
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
            let Some(family_id) = outcome.candidate_family_id.clone() else {
                continue;
            };
            family_hypotheses
                .entry(family_id.clone())
                .or_default()
                .insert(signature.clone());
            let family_size = outcome.candidate_family_size.unwrap();
            let stored_family_size = family_sizes
                .entry(family_id.clone())
                .or_insert(Some(family_size));
            if *stored_family_size != Some(family_size) {
                *stored_family_size = None;
            }
            candidates.entry((family_id.clone(), signature)).or_insert((
                candidate_parameters,
                baseline,
                specs,
                family_id,
            ));
        }
    }

    let mut family_corrections = BTreeMap::new();
    for (family_id, hypotheses) in family_hypotheses {
        let is_triggering_family = triggering_family_id == Some(family_id.as_str());
        if let Some(correction) = record_scout_promotion_look(
            state,
            &family_id,
            &hypotheses,
            family_sizes.get(&family_id).copied().flatten(),
            if is_triggering_family {
                triggering_event_id
            } else {
                None
            },
            if is_triggering_family {
                triggering_block_id
            } else {
                None
            },
        ) {
            family_corrections.insert(family_id, correction);
        }
    }

    let profile_key = key(symbol, strategy);
    for (_, (candidate_parameters, baseline, specs, family_id)) in candidates {
        let Some((look_count, candidate_count)) = family_corrections.get(&family_id).copied()
        else {
            continue;
        };
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
        let paired = paired_parameter_time_blocks(
            &state.variants,
            symbol,
            strategy,
            &candidate_parameters,
            &specs,
            &baseline,
            &family_id,
        );
        let base_alpha = if specs.iter().any(|spec| spec.key == "stop_loss_percent") {
            0.2
        } else {
            0.1
        };
        let Some(alpha) = scout_promotion_alpha(base_alpha, look_count, candidate_count) else {
            continue;
        };
        let bootstrap_replicates = promotion_bootstrap_replicates(alpha);
        let lower_bound = (paired.len() >= minimum_parameter_samples)
            .then_some(bootstrap_replicates)
            .flatten()
            .and_then(|replicates| {
                stationary_bootstrap_lower_bound(
                    &paired,
                    alpha,
                    MEAN_BLOCK_LENGTH_ONLINE_WINDOWS,
                    replicates,
                )
            });
        if !lower_bound.is_some_and(|bound| bound > 0.0) {
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

pub(super) fn scout_promotion_alpha(
    base_alpha: f64,
    look_count: u64,
    candidate_count: usize,
) -> Option<f64> {
    if !base_alpha.is_finite() || !(0.0..0.5).contains(&base_alpha) || look_count == 0 {
        return None;
    }
    let candidate_count = candidate_count.max(1) as f64;
    let looks = look_count as f64;
    let alpha = base_alpha / (candidate_count * looks * (looks + 1.0));
    (alpha.is_finite() && alpha > 0.0).then_some(alpha)
}

fn record_scout_promotion_look(
    state: &mut LearningState,
    family_id: &str,
    candidate_hypotheses: &BTreeSet<String>,
    family_hypothesis_count: Option<usize>,
    event_id: Option<&str>,
    look_block_id: Option<u64>,
) -> Option<(u64, usize)> {
    if !state.scout_promotion_look_history.contains_key(family_id)
        && state.scout_promotion_look_history.len() >= MAX_SCOUT_PROMOTION_FAMILIES
    {
        state.scout_promotion_look_history_overflowed = true;
        return None;
    }
    let history = state
        .scout_promotion_look_history
        .entry(family_id.to_owned())
        .or_default();
    if family_hypothesis_count.is_none_or(|count| {
        count == 0
            || count > MAX_SCOUT_PROMOTION_HYPOTHESES_PER_FAMILY
            || count < candidate_hypotheses.len()
    }) {
        history.overflowed = true;
    } else if history
        .family_hypothesis_count
        .is_some_and(|count| Some(count) != family_hypothesis_count)
    {
        history.overflowed = true;
    } else {
        history.family_hypothesis_count = family_hypothesis_count;
    }
    for hypothesis in candidate_hypotheses {
        if !history.candidate_hypotheses.contains(hypothesis)
            && history.candidate_hypotheses.len() >= MAX_SCOUT_PROMOTION_HYPOTHESES_PER_FAMILY
        {
            history.overflowed = true;
            break;
        }
        history.candidate_hypotheses.insert(hypothesis.clone());
    }
    if let Some(block_id) = look_block_id {
        let new_look = match history.last_look_block_id {
            None => {
                history.look_count = 1;
                history.last_look_block_id = Some(block_id);
                true
            }
            Some(last) if block_id > last => {
                history.look_count = history.look_count.saturating_add(1);
                history.last_look_block_id = Some(block_id);
                true
            }
            Some(last) if block_id == last => false,
            Some(_) => {
                history.overflowed = true;
                false
            }
        };
        if new_look && let Some(event_id) = event_id {
            history.last_look_id = Some(format!(
                "scout-promotion-look:v1:{}:{}:{block_id}:{event_id}",
                family_id, history.look_count,
            ));
        }
    }
    (!history.overflowed && history.look_count > 0).then_some((
        history.look_count,
        history.family_hypothesis_count.unwrap_or_default(),
    ))
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
            let parameters = variant.profile.parameters.clone();
            let mut valid_baseline = true;
            for spec in specs {
                if outcome.baseline_parameters.get(&spec.key).is_none() {
                    valid_baseline = false;
                    break;
                }
                // Baseline parameters are validated but not inserted —
                // the candidate parameters from the variant are used.
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
            None,
            None,
            None,
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
) -> Result<(ProfileOutcomeUpdate, bool), (StatusCode, String)> {
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
    let outcome_history_compacted = retention::compact_outcome_working_set(&mut state.variants);
    if outcome_history_compacted {
        state.outcome_history_truncated = true;
        rebuild_trade_profile_metrics(state, now_ms());
    }
    Ok((update, outcome_history_compacted))
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
        independence_block_id: event_independence_block_id(event),
        candidate_family_id: event_candidate_family_id(event),
        candidate_family_size: event_candidate_family_size(event),
        learning_context: match event {
            FeedbackEvent::ScoutOutcome {
                learning_context, ..
            } => learning_context.clone(),
            FeedbackEvent::TradeOutcome { scout: false, .. } => event_context(event)
                .and_then(|context| context.strip_suffix(":trade").map(str::to_owned)),
            _ => None,
        },
        market_regime: match event {
            FeedbackEvent::ScoutOutcome { market_regime, .. } => market_regime.clone(),
            _ => None,
        },
        horizon_seconds: match event {
            FeedbackEvent::ScoutOutcome {
                comparison:
                    ScoutComparisonMetadata::Paired {
                        horizon_seconds, ..
                    },
                ..
            } => *horizon_seconds,
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
        lineage: match event {
            FeedbackEvent::TradeOutcome { replay_context, .. } => replay_context
                .as_ref()
                .and_then(|context| context.lineage.clone()),
            FeedbackEvent::ScoutOutcome { .. }
            | FeedbackEvent::MissedOpportunityCandidate { .. } => None,
        },
    }
}

fn evidence_gain_scope(
    state: &LearningState,
    event: &FeedbackEvent,
) -> (Option<String>, Option<String>, Option<u32>) {
    let (symbol, strategy, profile_id) = match event {
        FeedbackEvent::MissedOpportunityCandidate {
            symbol,
            strategy,
            profile_id,
            ..
        }
        | FeedbackEvent::ScoutOutcome {
            symbol,
            strategy,
            profile_id,
            ..
        }
        | FeedbackEvent::TradeOutcome {
            symbol,
            strategy,
            profile_id,
            ..
        } => (symbol, strategy, profile_id),
    };
    let request_id = match event {
        FeedbackEvent::ScoutOutcome {
            market_regime: Some(regime),
            comparison:
                ScoutComparisonMetadata::Paired {
                    horizon_seconds: Some(horizon),
                    ..
                },
            learning_context: Some(context),
            ..
        } => scout_context_direction(context, symbol, strategy, regime).map(|direction| {
            evidence_request_key_with_direction(
                symbol,
                strategy,
                profile_id,
                regime,
                &horizon.to_string(),
                Some(direction),
            )
        }),
        FeedbackEvent::TradeOutcome {
            replay_context: Some(context),
            ..
        } => context
            .lineage
            .as_ref()
            .and_then(|lineage| lineage.evidence_gain_correlation.as_deref())
            .filter(|id| state.evidence_requests.contains_key(*id))
            .map(str::to_owned),
        _ => None,
    }
    .filter(|id| state.evidence_requests.contains_key(id));
    let cohort_id = request_id.as_ref().and_then(|id| {
        state
            .evidence_requests
            .get(id)
            .map(evidence_request_cohort_id)
    });
    let attempt = request_id
        .as_ref()
        .and_then(|id| state.evidence_requests.get(id))
        .map(|request| request.attempts);
    (cohort_id, request_id, attempt)
}

fn record_evidence_gain_event(
    state: &mut LearningState,
    event: &FeedbackEvent,
    before: EvidenceGainCoverageSnapshot,
    now_ms: u64,
) -> bool {
    let (event_id, symbol, strategy, _, profile_id) = event_parts(event);
    let (cohort_id, request_id, request_attempt) = evidence_gain_scope(state, event);
    let targeted_exploratory_trade = match event {
        FeedbackEvent::TradeOutcome {
            scout,
            entry_policy,
            ..
        } => *scout || entry_policy.as_deref() == Some("exploratory"),
        _ => false,
    };
    if request_id.is_none() && !targeted_exploratory_trade {
        return false;
    }
    let after = evidence_gain_coverage_snapshot(state, cohort_id.as_deref(), now_ms);
    let (
        outcome_event_id,
        source_event_id,
        trade_id,
        episode_id,
        profile_hash,
        profile_version,
        execution_mode,
        cost_status,
        lineage_complete,
    ) = match event {
        FeedbackEvent::TradeOutcome {
            event_id,
            trade_id,
            replay_context,
            ..
        } => {
            let lineage = replay_context
                .as_ref()
                .and_then(|context| context.lineage.as_ref());
            let context = replay_context.as_ref();
            let cost_status = match (context, lineage) {
                (Some(context), Some(lineage))
                    if context.execution_costs_complete
                        && lineage.fees_complete
                        && lineage
                            .fee_currency
                            .as_deref()
                            .is_some_and(|currency| !currency.trim().is_empty()) =>
                {
                    "complete"
                }
                (Some(_), Some(_)) => "incomplete",
                _ => "unknown",
            };
            (
                lineage
                    .map(|lineage| lineage.outcome_event_id.clone())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| event_id.clone()),
                lineage.and_then(|lineage| lineage.source_event_id.clone()),
                lineage
                    .and_then(|lineage| lineage.trade_id.clone())
                    .or_else(|| trade_id.clone()),
                lineage.and_then(|lineage| lineage.episode_id.clone()),
                lineage.and_then(|lineage| lineage.profile_hash.clone()),
                lineage.and_then(|lineage| lineage.profile_version),
                lineage.and_then(|lineage| lineage.execution_mode.clone()),
                cost_status,
                lineage.is_some(),
            )
        }
        FeedbackEvent::ScoutOutcome { event_id, .. } => (
            event_id.clone(),
            Some(event_id.clone()),
            None,
            None,
            None,
            None,
            None,
            "unknown",
            false,
        ),
        FeedbackEvent::MissedOpportunityCandidate { event_id, .. } => (
            event_id.clone(),
            Some(event_id.clone()),
            None,
            None,
            None,
            None,
            None,
            "unknown",
            false,
        ),
    };
    let new_coverage_observed = matches!(before.status.as_str(), "missing" | "stale")
        && after.status == "fresh"
        && after.sample_count.unwrap_or_default() > before.sample_count.unwrap_or_default()
        && after.last_sample_timestamp_ms > before.last_sample_timestamp_ms
        && source_event_id.is_some();
    let mut blocked_reasons = Vec::new();
    if cohort_id.is_none() {
        blocked_reasons.push("exact_evidence_cohort_unknown".to_owned());
    }
    if request_id.is_none() {
        blocked_reasons.push("exact_evidence_request_unknown".to_owned());
    }
    if source_event_id.is_none() {
        blocked_reasons.push("authoritative_source_event_missing".to_owned());
    }
    if matches!(event, FeedbackEvent::TradeOutcome { .. }) && !lineage_complete {
        blocked_reasons.push("authoritative_trade_lineage_missing".to_owned());
    }
    if cost_status != "complete" {
        blocked_reasons.push(format!("execution_costs_{cost_status}"));
    }
    // No Learning input currently carries independent ledger reconciliation
    // for this transition, so this remains a durable blocker.
    let reconciliation_status = "unknown";
    blocked_reasons.push("reconciliation_unknown".to_owned());
    if !new_coverage_observed {
        blocked_reasons.push("no_observed_fresh_coverage_transition".to_owned());
    }
    let key_parts = [
        event_id.to_owned(),
        source_event_id.clone().unwrap_or_default(),
        trade_id.clone().unwrap_or_default(),
        episode_id.clone().unwrap_or_default(),
        profile_id.to_owned(),
        execution_mode.clone().unwrap_or_default(),
        cohort_id.clone().unwrap_or_default(),
        request_id.clone().unwrap_or_default(),
    ];
    let record_id = format!(
        "evidence-gain:v1:{}",
        key_parts
            .iter()
            .map(|part| format!("{}:{part}", part.len()))
            .collect::<Vec<_>>()
            .join("|")
    );
    if state.evidence_gain_records.contains_key(&record_id) {
        return false;
    }
    let record = EvidenceGainRecord {
        schema_version: 1,
        record_id: record_id.clone(),
        outcome_event_id,
        source_event_id,
        trade_id,
        episode_id,
        profile_id: Some(profile_id.to_owned()),
        profile_hash,
        profile_version,
        execution_mode,
        evidence_cohort_id: cohort_id,
        evidence_request_id: request_id,
        evidence_request_attempt: request_attempt,
        observed_at_ms: now_ms,
        coverage_before_status: before.status,
        coverage_after_status: after.status,
        sample_count_before: before.sample_count,
        sample_count_after: after.sample_count,
        last_sample_before_ms: before.last_sample_timestamp_ms,
        last_sample_after_ms: after.last_sample_timestamp_ms,
        new_coverage_observed,
        request_closed_at_ms: None,
        cost_status: cost_status.to_owned(),
        reconciliation_status: reconciliation_status.to_owned(),
        eligibility_status: "blocked".to_owned(),
        blocked_reasons,
    };
    if state.evidence_gain_records.len() >= MAX_EVIDENCE_GAIN_RECORDS {
        let oldest_terminal = state
            .evidence_gain_records
            .iter()
            .filter(|(_, record)| {
                record.request_closed_at_ms.is_some() || record.evidence_request_id.is_none()
            })
            .min_by_key(|(_, record)| (record.observed_at_ms, record.record_id.clone()))
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest_terminal {
            state.evidence_gain_records.remove(&oldest);
        } else {
            let changed = !state.evidence_gain_records_overflowed;
            state.evidence_gain_records_overflowed = true;
            return changed;
        }
    }
    state.evidence_gain_records.insert(record_id, record);
    true
}

fn event_independence_block_id(event: &FeedbackEvent) -> Option<u64> {
    let FeedbackEvent::ScoutOutcome {
        comparison: ScoutComparisonMetadata::Paired { .. },
        ..
    } = event
    else {
        return None;
    };
    Some(event_observed_at(event) / SCOUT_PROMOTION_BLOCK_MS)
}

const SCOUT_PROMOTION_BLOCK_MS: u64 = 4 * 60 * 60 * 1_000;

fn event_candidate_family_id(event: &FeedbackEvent) -> Option<String> {
    let FeedbackEvent::ScoutOutcome {
        symbol,
        strategy,
        parameters: _,
        baseline_parameters,
        comparison: ScoutComparisonMetadata::Paired { .. },
        specs,
        ..
    } = event
    else {
        return None;
    };
    if baseline_parameters.is_empty() || specs.is_empty() {
        return None;
    }
    let tested_keys = specs
        .iter()
        .map(|spec| spec.key.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if tested_keys.len() != specs.len()
        || tested_keys
            .iter()
            .any(|key| !baseline_parameters.contains_key(*key))
    {
        return None;
    }
    let baseline_profile_id = strategy::profile_id(symbol, strategy, baseline_parameters);
    Some(format!(
        "scout-promotion-family:v1:{baseline_profile_id}:{}",
        tested_keys.join(",")
    ))
}

fn event_candidate_family_size(event: &FeedbackEvent) -> Option<usize> {
    let FeedbackEvent::ScoutOutcome {
        baseline_parameters,
        comparison: ScoutComparisonMetadata::Paired { .. },
        specs,
        ..
    } = event
    else {
        return None;
    };
    if baseline_parameters.is_empty() || specs.is_empty() {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut combinations = 1usize;
    for spec in specs {
        if !seen.insert(spec.key.as_str())
            || !baseline_parameters.contains_key(&spec.key)
            || !spec.min.is_finite()
            || !spec.max.is_finite()
            || !spec.step.is_finite()
            || spec.max < spec.min
            || spec.step <= 0.0
        {
            return None;
        }
        let step_count = (spec.max - spec.min) / spec.step;
        let rounded_step_count = step_count.round();
        if !step_count.is_finite()
            || (step_count - rounded_step_count).abs() > 1.0e-6 * step_count.abs().max(1.0)
        {
            return None;
        }
        let values = (rounded_step_count as usize).saturating_add(1);
        combinations = combinations.checked_mul(values)?;
        if combinations > MAX_SCOUT_PROMOTION_HYPOTHESES_PER_FAMILY {
            return None;
        }
    }
    Some(combinations)
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
    profile_key: &str,
    update: &ProfileOutcomeUpdate,
) {
    if matches!(event, FeedbackEvent::TradeOutcome { .. })
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

#[cfg(test)]
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

fn paired_parameter_time_blocks(
    variants: &BTreeMap<String, RegisteredProfile>,
    symbol: &str,
    strategy: &str,
    candidate_parameters: &BTreeMap<String, f64>,
    specs: &[ParameterSpec],
    baseline_parameters: &BTreeMap<String, f64>,
    candidate_family_id: &str,
) -> Vec<(u64, f64)> {
    let mut grouped = BTreeMap::<String, (u64, Vec<f64>)>::new();
    for variant in variants.values().filter(|variant| {
        variant.profile.symbol == symbol
            && variant.profile.strategy == strategy
            && variant.profile.parameters == *candidate_parameters
    }) {
        for outcome in variant.outcomes.iter().filter(|outcome| {
            outcome.source == LearningOutcomeSource::Scout
                && outcome.paired_comparison
                && outcome.baseline_parameters == *baseline_parameters
                && outcome.candidate_family_id.as_deref() == Some(candidate_family_id)
                && outcome.independence_block_id.is_some()
                && specs
                    .iter()
                    .all(|spec| outcome.tested_parameter_keys.contains(&spec.key))
        }) {
            let Some(group_id) = outcome.evidence_group_id.as_ref() else {
                continue;
            };
            let block_id = outcome.independence_block_id.unwrap();
            let entry = grouped
                .entry(group_id.clone())
                .or_insert_with(|| (block_id, Vec::new()));
            // Horizon arms for one comparison group share one UTC block. If
            // late arms cross a block boundary, attach the group to its first
            // observed block so it remains one sample.
            entry.0 = entry.0.min(block_id);
            entry.1.push(outcome.net_return_percent);
        }
    }
    let mut by_block = BTreeMap::<u64, Vec<f64>>::new();
    for (_, (block_id, returns)) in grouped {
        if returns.is_empty() || returns.iter().any(|value| !value.is_finite()) {
            continue;
        }
        by_block
            .entry(block_id)
            .or_default()
            .push(returns.iter().sum::<f64>() / returns.len() as f64);
    }
    by_block
        .into_iter()
        .map(|(block, values)| (block, values.iter().sum::<f64>() / values.len() as f64))
        .collect()
}

#[cfg(test)]
fn paired_candidate_promotion_lower_bound(
    outcomes: &[f64],
    specs: &[ParameterSpec],
) -> Option<f64> {
    if specs.iter().any(|spec| spec.key == "stop_loss_percent") {
        one_sided_80_lower_bound(outcomes)
    } else {
        one_sided_90_lower_bound(outcomes)
    }
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
#[cfg(test)]
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
#[cfg(test)]
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

#[cfg(test)]
fn one_sided_80_lower_bound(outcomes: &[f64]) -> Option<f64> {
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
    let critical = one_sided_80_t_critical(sample_count)?;
    Some(mean - critical * (variance / sample_count as f64).sqrt())
}

#[cfg(test)]
fn one_sided_80_t_critical(sample_count: usize) -> Option<f64> {
    let degrees = sample_count.checked_sub(1)? as f64;
    if degrees < 1.0 {
        return None;
    }
    match degrees as usize {
        1 => return Some(1.376_381_920_471_173_4),
        2 => return Some(1.060_660_171_779_821_4),
        3 => return Some(0.978_472_312_363_304_6),
        4 => return Some(0.940_964_577_189_989_1),
        _ => {}
    }
    let z = 0.841_621_233_572_914_3;
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
    if let Some(updates) = &state.state_updates {
        updates.notify_one();
    }
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

    fn producer_shaped_missed_opportunity() -> FeedbackEvent {
        let symbol = "BTC-USDT";
        let strategy = "live-momentum";
        let parameters = BTreeMap::from([("minimum_momentum".into(), 0.002)]);
        FeedbackEvent::MissedOpportunityCandidate {
            event_id: "missed:BTC-USDT:live-momentum:1000".into(),
            observed_at_ms: 1_000,
            symbol: symbol.into(),
            strategy: strategy.into(),
            profile_id: strategy::profile_id(symbol, strategy, &parameters),
            parameters,
            direction: "bullish".into(),
            strength: 0.61,
            score: 0.31,
            learning_context: "long:trend:BTC-USDT:live-momentum".into(),
        }
    }

    fn versioned_exploratory_trade(
        event_id: &str,
        source_event_id: &str,
        complete: bool,
    ) -> FeedbackEvent {
        let lineage = complete.then(|| strategy::TradeOutcomeLineage {
            outcome_event_id: event_id.into(),
            source_event_id: Some(source_event_id.into()),
            episode_id: Some("episode:thesis-7".into()),
            opportunity_id: Some("opportunity:8".into()),
            trade_id: Some("trade:9".into()),
            profile_hash: Some("sha256:entry-profile".into()),
            profile_version: Some(4),
            execution_mode: Some("paper".into()),
            entry_policy: Some("exploratory".into()),
            source: Some("auto_trader_entry_reservation".into()),
            entry_fill_ids: vec!["pm-fill:entry-1".into()],
            exit_fill_ids: vec!["pm-fill:exit-1".into()],
            fee_currency: Some("USDT".into()),
            fees_complete: true,
            evidence_gain_correlation: Some("gain-group-11".into()),
        });
        FeedbackEvent::TradeOutcome {
            event_id: event_id.into(),
            schema_version: Some(1),
            trade_id: Some("trade:9".into()),
            opportunity_id: Some("opportunity:8".into()),
            episode_id: Some("episode:thesis-7".into()),
            profile_hash: Some("sha256:entry-profile".into()),
            profile_version: Some(4),
            execution_mode: Some("paper".into()),
            entry_policy: Some("exploratory".into()),
            source: Some("auto_trader_entry_reservation".into()),
            observed_at_ms: 10,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile".into(),
            entry_profile_id: Some("entry-profile".into()),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: true,
            realized_pnl_usd: 0.01,
            net_return_percent: 0.1,
            threshold_observations: vec![],
            replay_context: Some(TradeReplayContext {
                market_symbol: "BTC-USDT".into(),
                entry_timestamp_ms: 1,
                exit_timestamp_ms: 2,
                side: "long".into(),
                entry_price: 10.0,
                exit_price: 11.0,
                quantity: 1.0,
                entry_fees: 0.01,
                exit_fees: 0.01,
                entry_walk_forward_evidence: None,
                execution_costs_complete: true,
                lineage,
            }),
        }
    }

    #[test]
    fn exploratory_lineage_keeps_source_events_distinct_from_the_same_thesis_episode() {
        let first = versioned_exploratory_trade("trade:9", "market-event:1", true);
        let second = versioned_exploratory_trade("trade:10", "market-event:2", true);
        assert!(validate_trade_replay_context(&first).is_ok());
        assert!(validate_trade_replay_context(&second).is_ok());
        let (
            FeedbackEvent::TradeOutcome {
                event_id: first_id,
                replay_context: Some(first_context),
                ..
            },
            FeedbackEvent::TradeOutcome {
                event_id: second_id,
                replay_context: Some(second_context),
                ..
            },
        ) = (first, second)
        else {
            panic!("trade events expected")
        };
        let first_lineage = first_context.lineage.unwrap();
        let second_lineage = second_context.lineage.unwrap();
        assert_ne!(first_id, second_id);
        assert_ne!(
            first_lineage.source_event_id,
            second_lineage.source_event_id
        );
        assert_eq!(first_lineage.episode_id, second_lineage.episode_id);
    }

    #[test]
    fn versioned_exploratory_feedback_quarantines_missing_identity_fills_or_fee_provenance() {
        let missing = versioned_exploratory_trade("trade:9", "market-event:1", false);
        let error = validate_trade_replay_context(&missing).unwrap_err();
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(error.1.contains("authoritative"));

        let mut non_scout_exploration = missing.clone();
        if let FeedbackEvent::TradeOutcome { scout, .. } = &mut non_scout_exploration {
            *scout = false;
        }
        assert_eq!(
            validate_trade_replay_context(&non_scout_exploration)
                .unwrap_err()
                .0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "exploratory policy must require lineage even when scout=false"
        );

        let mut incomplete_cost = versioned_exploratory_trade("trade:9", "market-event:1", true);
        if let FeedbackEvent::TradeOutcome {
            replay_context: Some(context),
            ..
        } = &mut incomplete_cost
        {
            let lineage = context.lineage.as_mut().unwrap();
            lineage.fee_currency = None;
            lineage.fees_complete = false;
        }
        assert_eq!(
            validate_trade_replay_context(&incomplete_cost)
                .unwrap_err()
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    fn scoped_scout_event(event_id: &str, observed_at_ms: u64) -> FeedbackEvent {
        FeedbackEvent::ScoutOutcome {
            event_id: event_id.into(),
            observed_at_ms,
            learning_context: Some("scout:v1:long:trend:BTC-USDT:live-momentum:trade".into()),
            market_regime: Some("trend".into()),
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile-1".into(),
            parameters: BTreeMap::new(),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Paired {
                group_id: format!("group-{event_id}"),
                horizon_seconds: Some(300),
            },
            specs: vec![],
            net_return_percent: 0.1,
        }
    }

    fn pending_evidence_request(now: u64) -> (String, EvidenceRequest) {
        let request_id = evidence_request_key_with_direction(
            "BTC-USDT",
            "live-momentum",
            "profile-1",
            "trend",
            "300",
            Some("long"),
        );
        (
            request_id,
            EvidenceRequest {
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                profile_id: "profile-1".into(),
                regime: "trend".into(),
                horizon: "300".into(),
                direction_scope: Some("long".into()),
                status: EvidenceRequestStatus::Running,
                attempts: 1,
                created_at_ms: now - 2_000,
                updated_at_ms: now - 1_000,
                next_attempt_at_ms: now,
                reason: None,
                coverage_before_status: Some("missing".into()),
                completion_source_event_id: None,
                completion_trade_id: None,
                completion_episode_id: None,
            },
        )
    }

    fn add_scout_outcome(state: &mut LearningState, event: &FeedbackEvent) {
        let (_, symbol, strategy, parameters, profile_id) = event_parts(event);
        let profile = StrategyProfile {
            profile_id: profile_id.clone(),
            version: 1,
            symbol: symbol.into(),
            strategy: strategy.into(),
            parameters: parameters.clone(),
            learning_samples: 1,
            positive_samples: 1,
            realized_pnl: 0.0,
        };
        let variant = RegisteredProfile {
            profile: profile.clone(),
            specs: vec![],
            samples: 1,
            positive: 1,
            realized_pnl: 0.0,
            outcomes: vec![learning_outcome(event)],
        };
        state.variants.insert(variant_key(&profile), variant);
    }

    #[test]
    fn evidence_gain_event_is_idempotent_across_sqlite_restart_and_closes_exact_request() {
        let now = 3 * 86_400_000;
        let event = scoped_scout_event("scout-event-1", now - 1_000);
        let (request_id, request) = pending_evidence_request(now);
        let mut state = LearningState::default();
        state.evidence_requests.insert(request_id.clone(), request);
        let before = evidence_gain_coverage_snapshot(
            &state,
            Some(
                "BTC-USDT|live-momentum|profile:profile-1|regime:trend|horizon:300|direction:long",
            ),
            now,
        );
        assert_eq!(before.status, "missing");
        add_scout_outcome(&mut state, &event);
        assert!(record_evidence_gain_event(
            &mut state,
            &event,
            before.clone(),
            now
        ));
        let record = state.evidence_gain_records.values().next().unwrap();
        assert!(record.new_coverage_observed);
        assert_eq!(record.sample_count_before, Some(0));
        assert_eq!(record.sample_count_after, Some(1));
        assert_eq!(record.eligibility_status, "blocked");
        assert_eq!(record.cost_status, "unknown");
        assert_eq!(record.reconciliation_status, "unknown");

        let path = std::env::temp_dir().join(format!(
            "learning-evidence-gain-restart-{}-{now}.sqlite",
            std::process::id()
        ));
        persist(&path, &state).unwrap();
        let mut restarted = super::super::load_state(&path).unwrap();
        assert_eq!(restarted.evidence_gain_records.len(), 1);
        assert!(!record_evidence_gain_event(
            &mut restarted,
            &event,
            before,
            now
        ));
        assert_eq!(restarted.evidence_gain_records.len(), 1);

        restarted
            .evidence_requests
            .get_mut(&request_id)
            .unwrap()
            .status = EvidenceRequestStatus::Completed;
        assert!(record_evidence_gain_request_completion(
            &mut restarted,
            &request_id,
            now + 1
        ));
        let linked = &restarted.evidence_requests[&request_id];
        assert_eq!(
            linked.completion_source_event_id.as_deref(),
            Some("scout-event-1")
        );
        assert!(!record_evidence_gain_request_completion(
            &mut restarted,
            &request_id,
            now + 2
        ));
        persist(&path, &restarted).unwrap();
        let recovered = super::super::load_state(&path).unwrap();
        assert_eq!(recovered.evidence_gain_records.len(), 1);
        assert_eq!(
            recovered.evidence_requests[&request_id]
                .completion_source_event_id
                .as_deref(),
            Some("scout-event-1")
        );
    }

    #[test]
    fn same_episode_trade_events_keep_distinct_blocked_gain_records() {
        let now = 4 * 86_400_000;
        let request_id = "gain-group-11".to_owned();
        let mut state = LearningState::default();
        let (_, mut request) = pending_evidence_request(now);
        request.status = EvidenceRequestStatus::Running;
        request.attempts = 1;
        state.evidence_requests.insert(request_id.clone(), request);
        let before = EvidenceGainCoverageSnapshot {
            status: "missing".into(),
            sample_count: Some(0),
            last_sample_timestamp_ms: None,
        };
        for (event_id, source_event_id, trade_id) in [
            ("trade:event-1", "source-event-1", "trade-1"),
            ("trade:event-2", "source-event-2", "trade-2"),
        ] {
            let mut event = versioned_exploratory_trade(event_id, source_event_id, true);
            if let FeedbackEvent::TradeOutcome {
                trade_id: event_trade,
                replay_context: Some(context),
                ..
            } = &mut event
            {
                *event_trade = Some(trade_id.into());
                let lineage = context.lineage.as_mut().unwrap();
                lineage.trade_id = Some(trade_id.into());
                lineage.episode_id = Some("episode:shared".into());
                lineage.evidence_gain_correlation = Some(request_id.clone());
            }
            assert!(record_evidence_gain_event(
                &mut state,
                &event,
                before.clone(),
                now
            ));
        }
        assert_eq!(state.evidence_gain_records.len(), 2);
        let records = state.evidence_gain_records.values().collect::<Vec<_>>();
        assert_eq!(records[0].episode_id.as_deref(), Some("episode:shared"));
        assert_eq!(records[1].episode_id.as_deref(), Some("episode:shared"));
        assert_ne!(records[0].source_event_id, records[1].source_event_id);
        assert_ne!(records[0].trade_id, records[1].trade_id);
        assert!(records.iter().all(|record| {
            record.eligibility_status == "blocked"
                && record.reconciliation_status == "unknown"
                && !record.new_coverage_observed
        }));
    }

    #[test]
    fn missing_lineage_or_incomplete_fees_remains_blocked_without_numeric_gain() {
        let now = 5 * 86_400_000;
        let mut state = LearningState::default();
        let mut missing = versioned_exploratory_trade("trade:missing", "source:missing", false);
        if let FeedbackEvent::TradeOutcome { schema_version, .. } = &mut missing {
            *schema_version = None;
        }
        assert!(record_evidence_gain_event(
            &mut state,
            &missing,
            EvidenceGainCoverageSnapshot {
                status: "unknown".into(),
                sample_count: None,
                last_sample_timestamp_ms: None,
            },
            now
        ));
        let missing_record = state.evidence_gain_records.values().next().unwrap();
        assert_eq!(missing_record.eligibility_status, "blocked");
        assert_eq!(missing_record.cost_status, "unknown");
        assert!(
            missing_record
                .blocked_reasons
                .contains(&"authoritative_trade_lineage_missing".to_owned())
        );
        assert!(!missing_record.new_coverage_observed);

        let mut incomplete = versioned_exploratory_trade("trade:fees", "source:fees", true);
        if let FeedbackEvent::TradeOutcome {
            replay_context: Some(context),
            ..
        } = &mut incomplete
        {
            context.execution_costs_complete = false;
            context.lineage.as_mut().unwrap().fees_complete = false;
            context.lineage.as_mut().unwrap().evidence_gain_correlation = None;
        }
        assert!(record_evidence_gain_event(
            &mut state,
            &incomplete,
            EvidenceGainCoverageSnapshot {
                status: "stale".into(),
                sample_count: Some(1),
                last_sample_timestamp_ms: Some(now - 100_000),
            },
            now
        ));
        let incomplete_record = state
            .evidence_gain_records
            .values()
            .find(|record| record.outcome_event_id == "trade:fees")
            .unwrap();
        assert_eq!(incomplete_record.cost_status, "incomplete");
        assert_eq!(incomplete_record.eligibility_status, "blocked");
        assert!(
            incomplete_record
                .blocked_reasons
                .contains(&"reconciliation_unknown".to_owned())
        );
    }

    #[test]
    fn legacy_trade_feedback_without_exploratory_lineage_remains_decodable() {
        let legacy = serde_json::json!({
            "kind":"trade_outcome", "event_id":"trade:legacy", "observed_at_ms":1,
            "symbol":"BTC-USDT", "strategy":"live-momentum", "profile_id":"legacy-profile",
            "parameters":{}, "specs":[], "scout":false, "realized_pnl_usd":0.0,
            "replay_context": {
                "market_symbol":"BTC-USDT", "entry_timestamp_ms":1, "exit_timestamp_ms":2,
                "side":"long", "entry_price":10.0, "exit_price":11.0, "quantity":1.0,
                "entry_fees":0.0, "exit_fees":0.0, "execution_costs_complete":false
            }
        });
        let event: FeedbackEvent = serde_json::from_value(legacy).unwrap();
        assert!(validate_trade_replay_context(&event).is_ok());
    }

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
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let event = producer_shaped_missed_opportunity();
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
        assert_eq!(
            job.profile_id,
            strategy::profile_id("BTC-USDT", "live-momentum", &job.parameters)
        );
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

    #[tokio::test]
    async fn missed_opportunity_validation_reports_the_invalid_field() {
        let invalid_events: [(&str, fn(&mut FeedbackEvent)); 10] = [
            ("event_id", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate { event_id, .. } = event {
                    event_id.clear();
                }
            }),
            ("strategy", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate { strategy, .. } = event {
                    *strategy = "scout-probe".into();
                }
            }),
            ("profile_id", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate { profile_id, .. } = event {
                    profile_id.clear();
                }
            }),
            ("parameters.count", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate { parameters, .. } = event {
                    for index in 0..65 {
                        parameters.insert(format!("p{index}"), 0.1);
                    }
                }
            }),
            ("parameters.key_length", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate { parameters, .. } = event {
                    parameters.insert("x".repeat(129), 0.1);
                }
            }),
            ("parameters.value", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate { parameters, .. } = event {
                    parameters.insert("minimum_momentum".into(), f64::NAN);
                }
            }),
            ("direction", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate { direction, .. } = event {
                    *direction = "sideways".into();
                }
            }),
            ("strength", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate { strength, .. } = event {
                    *strength = f64::INFINITY;
                }
            }),
            ("score", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate { score, .. } = event {
                    *score = f64::NAN;
                }
            }),
            ("learning_context", |event: &mut FeedbackEvent| {
                if let FeedbackEvent::MissedOpportunityCandidate {
                    learning_context, ..
                } = event
                {
                    *learning_context = "x".repeat(257);
                }
            }),
        ];
        for (field, mutate) in invalid_events {
            let service = ServiceState {
                state: Arc::new(Mutex::new(LearningState::default())),
                path: std::env::temp_dir().join(format!(
                    "learning-invalid-{field}-{}.sqlite",
                    std::process::id()
                )),
                state_updates: Arc::new(tokio::sync::Notify::new()),
            };
            let mut event = producer_shaped_missed_opportunity();
            mutate(&mut event);
            let error = accept_feedback(State(service), Json(event))
                .await
                .err()
                .unwrap();
            assert_eq!(error.0, StatusCode::BAD_REQUEST, "field {field}");
            assert!(error.1.contains(field), "field {field}: {}", error.1);
        }

        let service = ServiceState {
            state: Arc::new(Mutex::new(LearningState::default())),
            path: std::env::temp_dir().join(format!(
                "learning-invalid-symbol-{}.sqlite",
                std::process::id()
            )),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let mut invalid_symbol = producer_shaped_missed_opportunity();
        if let FeedbackEvent::MissedOpportunityCandidate { symbol, .. } = &mut invalid_symbol {
            *symbol = "NOT-ALLOWED".into();
        }
        let error = accept_feedback(State(service), Json(invalid_symbol))
            .await
            .err()
            .unwrap();
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(error.1.contains("symbol"));

        let service = ServiceState {
            state: Arc::new(Mutex::new(LearningState::default())),
            path: std::env::temp_dir()
                .join(format!("learning-oversized-{}.sqlite", std::process::id())),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let mut oversized = producer_shaped_missed_opportunity();
        if let FeedbackEvent::MissedOpportunityCandidate {
            learning_context, ..
        } = &mut oversized
        {
            *learning_context = "x".repeat(17 * 1024);
        }
        let error = accept_feedback(State(service), Json(oversized))
            .await
            .err()
            .unwrap();
        assert_eq!(error.0, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(error.1.contains("16 KiB"));
    }

    #[test]
    fn trade_feedback_creates_one_durable_replay_job_and_missing_context_is_unsupported() {
        let event = FeedbackEvent::TradeOutcome {
            event_id: "trade:missing-context".into(),
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
            schema_version: Some(1),
            trade_id: Some("trade-1".into()),
            opportunity_id: Some("opportunity-1".into()),
            episode_id: Some("episode-1".into()),
            profile_hash: Some("sha256:profile".into()),
            profile_version: Some(17),
            execution_mode: Some("paper".into()),
            entry_policy: Some("post_only".into()),
            source: Some("auto_trader".into()),
            entry_profile_id: Some("entry-profile".into()),
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
                lineage: Some(strategy::TradeOutcomeLineage {
                    outcome_event_id: "trade:replay".into(),
                    source_event_id: Some("market-event:41".into()),
                    episode_id: Some("episode-1".into()),
                    opportunity_id: Some("opportunity-1".into()),
                    trade_id: Some("trade-1".into()),
                    profile_hash: Some("sha256:profile".into()),
                    profile_version: Some(17),
                    execution_mode: Some("paper".into()),
                    entry_policy: Some("post_only".into()),
                    source: Some("auto_trader".into()),
                    entry_fill_ids: vec!["pm-fill:entry".into()],
                    exit_fill_ids: vec!["pm-fill:exit".into()],
                    fee_currency: Some("USDT".into()),
                    fees_complete: true,
                    evidence_gain_correlation: Some("gain:41".into()),
                }),
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
        assert_eq!(loaded_job.schema_version, Some(1));
        assert_eq!(loaded_job.trade_id.as_deref(), Some("trade-1"));
        assert_eq!(loaded_job.opportunity_id.as_deref(), Some("opportunity-1"));
        assert_eq!(loaded_job.episode_id.as_deref(), Some("episode-1"));
        assert_eq!(
            loaded_job.entry_profile_id.as_deref(),
            Some("entry-profile")
        );
        assert_eq!(loaded_job.profile_id, "profile");
        assert_eq!(loaded_job.profile_hash.as_deref(), Some("sha256:profile"));
        assert_eq!(loaded_job.profile_version, Some(17));
        assert_eq!(loaded_job.execution_mode.as_deref(), Some("paper"));
        assert_eq!(loaded_job.entry_policy.as_deref(), Some("post_only"));
        assert_eq!(loaded_job.source.as_deref(), Some("auto_trader"));
        let lineage = loaded_job
            .context
            .as_ref()
            .unwrap()
            .lineage
            .as_ref()
            .unwrap();
        assert_eq!(lineage.outcome_event_id, "trade:replay");
        assert_eq!(lineage.source_event_id.as_deref(), Some("market-event:41"));
        assert_eq!(lineage.episode_id.as_deref(), Some("episode-1"));
        assert_eq!(lineage.entry_fill_ids, vec!["pm-fill:entry"]);
        assert_eq!(lineage.exit_fill_ids, vec!["pm-fill:exit"]);
        assert_eq!(lineage.fee_currency.as_deref(), Some("USDT"));
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

    #[tokio::test]
    async fn canonical_trade_feedback_survives_restart_and_duplicate_acceptance() {
        let path = std::env::temp_dir().join(format!(
            "learning-canonical-replay-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let failed_path = std::env::temp_dir().join(format!(
            "learning-canonical-replay-failure-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&failed_path);
        std::fs::create_dir_all(&failed_path).unwrap();
        let mut service = ServiceState {
            state: Arc::new(Mutex::new(LearningState::default())),
            // Opening a directory as the SQLite database deterministically
            // fails during persistence after the in-memory apply path runs.
            path: failed_path.clone(),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let event = FeedbackEvent::TradeOutcome {
            event_id: "trade:canonical".into(),
            schema_version: Some(1),
            trade_id: Some("trade-42".into()),
            opportunity_id: Some("opp-42".into()),
            episode_id: Some("episode-42".into()),
            profile_hash: Some("hash-42".into()),
            profile_version: Some(42),
            execution_mode: Some("paper".into()),
            entry_policy: Some("maker_only".into()),
            source: Some("position_ledger".into()),
            entry_profile_id: Some("profile-at-entry".into()),
            observed_at_ms: 2_000,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "derived-training-profile".into(),
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
                lineage: None,
            }),
        };

        let failed = accept_feedback(State(service.clone()), Json(event.clone())).await;
        assert!(failed.is_err());
        {
            let state = service.state.lock().await;
            assert!(!state.seen_event_ids.contains("trade:canonical"));
            assert_eq!(state.accepted_events, 0);
            assert!(state.strategy_replay_jobs.is_empty());
            assert!(state.variants.is_empty());
        }

        // Retry the exact event in the same process after storage recovers.
        // It must be applied and committed rather than reported as duplicate.
        service.path = path.clone();
        let accepted = accept_feedback(State(service), Json(event.clone()))
            .await
            .unwrap();
        assert!(accepted.0.accepted);
        assert!(!accepted.0.duplicate);

        let restarted = super::super::service_state(path.clone()).unwrap();
        let before = restarted.state.lock().await.clone();
        let job = before.strategy_replay_jobs.get("trade:canonical").unwrap();
        assert_eq!(job.entry_profile_id.as_deref(), Some("profile-at-entry"));
        assert_eq!(job.profile_id, "derived-training-profile");
        assert_eq!(job.trade_id.as_deref(), Some("trade-42"));
        assert_eq!(job.profile_version, Some(42));
        let profile_statistics_before = before
            .variants
            .iter()
            .map(|(key, profile)| {
                (
                    key.clone(),
                    profile.samples,
                    profile.positive,
                    profile.realized_pnl,
                    profile.outcomes.len(),
                )
            })
            .collect::<Vec<_>>();
        let accepted_events_before = before.accepted_events;
        drop(before);

        let duplicate = accept_feedback(State(restarted.clone()), Json(event))
            .await
            .unwrap();
        assert!(duplicate.0.accepted);
        assert!(duplicate.0.duplicate);
        let after = restarted.state.lock().await;
        assert_eq!(after.strategy_replay_jobs.len(), 1);
        let profile_statistics_after = after
            .variants
            .iter()
            .map(|(key, profile)| {
                (
                    key.clone(),
                    profile.samples,
                    profile.positive,
                    profile.realized_pnl,
                    profile.outcomes.len(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(profile_statistics_after, profile_statistics_before);
        assert_eq!(after.accepted_events, accepted_events_before);

        drop(after);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_dir_all(failed_path);
    }

    #[test]
    fn legacy_replay_job_json_defaults_canonical_lineage_to_none() {
        let event = FeedbackEvent::TradeOutcome {
            event_id: "trade:legacy".into(),
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
            profile_id: "legacy-profile".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: 0.0,
            net_return_percent: 0.0,
            threshold_observations: vec![],
            replay_context: None,
        };
        let mut state = LearningState::default();
        enqueue_strategy_replay(&mut state, &event);
        let mut legacy_json =
            serde_json::to_value(state.strategy_replay_jobs.get("trade:legacy").unwrap()).unwrap();
        for field in [
            "schema_version",
            "trade_id",
            "opportunity_id",
            "episode_id",
            "entry_profile_id",
            "profile_hash",
            "profile_version",
            "execution_mode",
            "entry_policy",
            "source",
        ] {
            legacy_json.as_object_mut().unwrap().remove(field);
        }
        let legacy: StrategyReplayJob = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(legacy.schema_version, None);
        assert_eq!(legacy.trade_id, None);
        assert_eq!(legacy.opportunity_id, None);
        assert_eq!(legacy.episode_id, None);
        assert_eq!(legacy.entry_profile_id, None);
        assert_eq!(legacy.profile_hash, None);
        assert_eq!(legacy.profile_version, None);
        assert_eq!(legacy.execution_mode, None);
        assert_eq!(legacy.entry_policy, None);
        assert_eq!(legacy.source, None);
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

    #[test]
    fn stop_loss_scout_successes_use_a_lower_promotion_bound_than_other_parameters() {
        let mut paired = vec![0.1; MINIMUM_SCOUT_PARAMETER_SAMPLES - 1];
        paired.push(-0.4);
        assert!(one_sided_90_lower_bound(&paired).unwrap() < 0.0);
        assert!(one_sided_80_lower_bound(&paired).unwrap() > 0.0);

        let stop_spec = ParameterSpec {
            key: "stop_loss_percent".into(),
            label: "Stop loss".into(),
            default: 0.5,
            min: 0.1,
            max: 1.5,
            step: 0.05,
            unit: "%".into(),
            learning_direction: -1.0,
        };
        assert!(paired_candidate_promotion_lower_bound(&paired, &[stop_spec]).unwrap() > 0.0);
        assert!(paired_candidate_promotion_lower_bound(&paired, &[spec()]).unwrap() < 0.0);
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
            market_regime: Some("trend".into()),
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
    fn paired_scout_outcomes_get_versioned_block_and_candidate_family_metadata() {
        let mut event = scout_event();
        if let FeedbackEvent::ScoutOutcome {
            observed_at_ms,
            learning_context,
            parameters,
            baseline_parameters,
            comparison,
            ..
        } = &mut event
        {
            *observed_at_ms = 12 * 86_400_000 + 10;
            *learning_context = Some("scout:v1:long:trend:BTC-USDT:momentum:trade".into());
            parameters.insert("threshold".into(), 0.4);
            baseline_parameters.insert("threshold".into(), 0.5);
            *comparison = ScoutComparisonMetadata::Paired {
                group_id: "comparison-1".into(),
                horizon_seconds: Some(300),
            };
        }
        assert_eq!(event_independence_block_id(&event), Some(72));
        let family = event_candidate_family_id(&event).unwrap();
        assert!(family.starts_with("scout-promotion-family:v1:"));
        let outcome = learning_outcome(&event);
        assert_eq!(outcome.independence_block_id, Some(72));
        assert_eq!(
            outcome.candidate_family_id.as_deref(),
            Some(family.as_str())
        );
        assert_eq!(outcome.candidate_family_size, Some(11));
        assert_eq!(
            outcome.learning_context.as_deref(),
            Some("scout:v1:long:trend:BTC-USDT:momentum:trade")
        );
        let mut alternate_candidate = event.clone();
        if let FeedbackEvent::ScoutOutcome { parameters, .. } = &mut alternate_candidate {
            parameters.insert("threshold".into(), 0.3);
        }
        assert_eq!(
            event_candidate_family_id(&alternate_candidate),
            Some(family)
        );
    }

    #[test]
    fn oversized_candidate_grid_has_no_promotable_family_size() {
        let mut event = scout_event();
        if let FeedbackEvent::ScoutOutcome {
            baseline_parameters,
            comparison,
            specs,
            ..
        } = &mut event
        {
            baseline_parameters.insert("threshold".into(), 0.5);
            *comparison = ScoutComparisonMetadata::Paired {
                group_id: "oversized-family".into(),
                horizon_seconds: Some(60),
            };
            specs[0].min = 0.0;
            specs[0].max = 1.0;
            specs[0].step = 0.001;
        }
        assert_eq!(event_candidate_family_size(&event), None);
        assert!(event_candidate_family_id(&event).is_some());
    }

    #[test]
    fn paired_scout_comparisons_collapse_horizons_and_same_block_groups() {
        let candidate_parameters = BTreeMap::from([("threshold".into(), 0.4)]);
        let baseline = BTreeMap::from([("threshold".into(), 0.5)]);
        let family = "family-v1";
        let outcome = |group: &str, block: u64, value: f64| LearningOutcome {
            observed_at_ms: block * 86_400_000,
            net_return_percent: value,
            evidence_group_id: Some(group.into()),
            independence_block_id: Some(block),
            candidate_family_id: Some(family.into()),
            candidate_family_size: Some(11),
            learning_context: None,
            market_regime: None,
            horizon_seconds: Some(300),
            source: LearningOutcomeSource::Scout,
            realized_pnl_usd: None,
            paired_comparison: true,
            tested_parameter_keys: vec!["threshold".into()],
            baseline_parameters: baseline.clone(),
            lineage: None,
        };
        let variant = RegisteredProfile {
            profile: StrategyProfile {
                profile_id: "candidate-profile".into(),
                version: 0,
                symbol: "BTC-USDT".into(),
                strategy: "momentum".into(),
                parameters: candidate_parameters.clone(),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            },
            specs: vec![spec()],
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            outcomes: vec![
                outcome("comparison-a", 10, 0.1),
                outcome("comparison-a", 11, 0.3),
                outcome("comparison-b", 10, 0.5),
                outcome("comparison-c", 12, 0.4),
                LearningOutcome {
                    independence_block_id: None,
                    candidate_family_id: None,
                    ..outcome("legacy", 12, 10.0)
                },
            ],
        };
        let variants = BTreeMap::from([("candidate".into(), variant)]);
        let blocks = paired_parameter_time_blocks(
            &variants,
            "BTC-USDT",
            "momentum",
            &candidate_parameters,
            &[spec()],
            &baseline,
            family,
        );
        assert_eq!(blocks, vec![(10, 0.35), (12, 0.4)]);
    }

    #[test]
    fn promotion_look_history_is_durable_bounded_and_more_conservative_over_time() {
        let mut state = LearningState::default();
        let hypotheses = BTreeSet::from(["candidate-a".into(), "candidate-b".into()]);
        assert_eq!(
            record_scout_promotion_look(
                &mut state,
                "family",
                &hypotheses,
                Some(4),
                Some("event-1"),
                Some(10),
            ),
            Some((1, 4))
        );
        assert_eq!(
            record_scout_promotion_look(
                &mut state,
                "family",
                &hypotheses,
                Some(4),
                Some("event-2"),
                Some(11),
            ),
            Some((2, 4))
        );
        assert_eq!(
            record_scout_promotion_look(
                &mut state,
                "family",
                &hypotheses,
                Some(4),
                Some("same-block-other-horizon"),
                Some(11),
            ),
            Some((2, 4))
        );
        let first_alpha = scout_promotion_alpha(0.1, 1, 4).unwrap();
        let later_alpha = scout_promotion_alpha(0.1, 2, 4).unwrap();
        assert!(later_alpha < first_alpha);
        let saved = serde_json::to_vec(&state).unwrap();
        let restored: LearningState = serde_json::from_slice(&saved).unwrap();
        let history = &restored.scout_promotion_look_history["family"];
        assert_eq!(history.look_count, 2);
        assert_eq!(history.last_look_block_id, Some(11));
        assert_eq!(history.family_hypothesis_count, Some(4));
        assert_eq!(history.candidate_hypotheses.len(), 2);
        assert!(history.last_look_id.as_deref().unwrap().contains("event-2"));
        let legacy: ScoutPromotionLookHistory = serde_json::from_value(serde_json::json!({
            "look_count": 2,
            "candidate_hypotheses": ["candidate-a"],
            "last_look_id": "legacy-look",
            "overflowed": false
        }))
        .unwrap();
        assert_eq!(legacy.family_hypothesis_count, None);
        assert_eq!(legacy.last_look_block_id, None);
    }

    #[test]
    fn promotion_look_history_fails_closed_on_reordered_blocks_or_changed_family_size() {
        let hypotheses = BTreeSet::from(["candidate-a".into()]);
        let mut reordered = LearningState::default();
        assert!(
            record_scout_promotion_look(
                &mut reordered,
                "family",
                &hypotheses,
                Some(4),
                Some("later"),
                Some(11),
            )
            .is_some()
        );
        assert!(
            record_scout_promotion_look(
                &mut reordered,
                "family",
                &hypotheses,
                Some(4),
                Some("late-arrival"),
                Some(10),
            )
            .is_none()
        );

        let mut changed_size = LearningState::default();
        assert!(
            record_scout_promotion_look(
                &mut changed_size,
                "family",
                &hypotheses,
                Some(4),
                Some("look-1"),
                Some(10),
            )
            .is_some()
        );
        assert!(
            record_scout_promotion_look(
                &mut changed_size,
                "family",
                &hypotheses,
                Some(5),
                Some("look-2"),
                Some(11),
            )
            .is_none()
        );
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
                independence_block_id: None,
                candidate_family_id: None,
                candidate_family_size: None,
                learning_context: None,
                market_regime: None,
                horizon_seconds: None,
                source: LearningOutcomeSource::Scout,
                realized_pnl_usd: None,
                paired_comparison: true,
                tested_parameter_keys: vec!["threshold".into()],
                baseline_parameters: baseline,
                lineage: None,
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
    fn paired_stop_and_target_evidence_promotes_only_the_symbol_strategy_profile() {
        let symbol = "ETH-USDT";
        let strategy_name = "live-momentum";
        let specs = vec![
            ParameterSpec {
                key: "stop_loss_percent".into(),
                label: "Stop loss".into(),
                default: 0.5,
                min: 0.5,
                max: 0.6,
                step: 0.05,
                unit: "%".into(),
                learning_direction: -1.0,
            },
            ParameterSpec {
                key: "take_profit_percent".into(),
                label: "Take profit".into(),
                default: 0.8,
                min: 0.8,
                max: 1.0,
                step: 0.1,
                unit: "%".into(),
                learning_direction: 1.0,
            },
        ];
        let baseline = BTreeMap::from([
            ("stop_loss_percent".into(), 0.5),
            ("take_profit_percent".into(), 0.8),
        ]);
        let candidate = BTreeMap::from([
            ("stop_loss_percent".into(), 0.6),
            ("take_profit_percent".into(), 1.0),
        ]);
        let candidate_profile = StrategyProfile {
            profile_id: "eth-risk-candidate".into(),
            version: 0,
            symbol: symbol.into(),
            strategy: strategy_name.into(),
            parameters: candidate.clone(),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let outcomes = (0..MINIMUM_SCOUT_PARAMETER_SAMPLES)
            .map(|index| LearningOutcome {
                observed_at_ms: index as u64 + 1,
                net_return_percent: 0.1,
                evidence_group_id: Some(format!("risk-probe-{index}")),
                independence_block_id: Some(index as u64),
                candidate_family_id: Some("family-risk".into()),
                candidate_family_size: Some(9),
                learning_context: None,
                market_regime: None,
                horizon_seconds: None,
                source: LearningOutcomeSource::Scout,
                realized_pnl_usd: None,
                paired_comparison: true,
                tested_parameter_keys: specs.iter().map(|spec| spec.key.clone()).collect(),
                baseline_parameters: baseline.clone(),
                lineage: None,
            })
            .collect();
        let registered = RegisteredProfile {
            profile: candidate_profile,
            specs: specs.clone(),
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            outcomes,
        };
        let mut variants = BTreeMap::new();
        variants.insert("eth-risk-candidate".into(), registered);

        let make_active = |profile_symbol: &str| StrategyProfile {
            profile_id: format!("{profile_symbol}-baseline"),
            version: 4,
            symbol: profile_symbol.into(),
            strategy: strategy_name.into(),
            parameters: baseline.clone(),
            learning_samples: 12,
            positive_samples: 7,
            realized_pnl: 1.25,
        };
        let active_profiles = BTreeMap::from([
            ("ETH-USDT:live-momentum".into(), make_active("ETH-USDT")),
            ("BTC-USDT:live-momentum".into(), make_active("BTC-USDT")),
        ]);
        let mut state = LearningState {
            variants,
            active_profiles,
            ..LearningState::default()
        };
        promote_qualified_scout_candidates(
            &mut state,
            symbol,
            strategy_name,
            MINIMUM_SCOUT_PARAMETER_SAMPLES,
            Some("scout-promotion-event"),
            Some("family-risk"),
            Some(9),
        );
        let promoted = &state.active_profiles["ETH-USDT:live-momentum"];
        assert_eq!(promoted.parameters["stop_loss_percent"], 0.6);
        assert_eq!(promoted.parameters["take_profit_percent"], 1.0);
        assert_eq!(promoted.version, 5);
        assert_eq!(
            state.active_profiles["BTC-USDT:live-momentum"].parameters,
            baseline
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
        assert_eq!(learning_outcome(&horizon).horizon_seconds, Some(300));
        let trade = FeedbackEvent::TradeOutcome {
            event_id: "t".into(),
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
            minimum_live_scout_parameter_samples: 10,
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
        assert_eq!(decoded.minimum_live_scout_parameter_samples, 10);
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
