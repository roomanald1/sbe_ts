use super::*;
use strategy::StrategyWalkForwardEvidence;

const OUTCOME_HALF_LIFE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const PARAMETER_HALF_LIFE_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

fn outcome_decay_weight(observed_at_ms: u64, now_ms: u64) -> f64 {
    let age_ms = now_ms.saturating_sub(observed_at_ms);
    0.5_f64.powf(age_ms as f64 / OUTCOME_HALF_LIFE_MS as f64)
}

pub(super) fn scout_context_direction<'a>(
    learning_context: &'a str,
    symbol: &str,
    strategy: &str,
    regime: &str,
) -> Option<&'a str> {
    let context = learning_context
        .strip_prefix("scout:")?
        .strip_suffix(":trade")?;
    let mut parts = context.split(':');
    let first = parts.next()?;
    let (direction, context_regime, context_symbol, context_strategy) = if first == "v1" {
        (parts.next()?, parts.next()?, parts.next()?, parts.next()?)
    } else {
        (first, parts.next()?, parts.next()?, parts.next()?)
    };
    (matches!(direction, "long" | "short")
        && context_regime == regime
        && context_symbol == symbol
        && context_strategy == strategy)
        .then_some(direction)
}

/// Compact Scout evidence for workers that need to schedule additional
/// observations. Keys include symbol/strategy aggregates and exact profiles.
pub(crate) fn walk_forward_priority_snapshot(
    state: &LearningState,
    now_ms: u64,
) -> BTreeMap<String, serde_json::Value> {
    const DEFAULT_MAX_EVIDENCE_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
    let mut cohorts = BTreeMap::<String, BTreeMap<u64, f64>>::new();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut last_samples = BTreeMap::<String, u64>::new();
    let mut max_age_ms_by_cohort = BTreeMap::<String, u64>::new();
    let mut seen = BTreeSet::<String>::new();
    let profile_max_age_ms = |profile: &StrategyProfile| {
        profile
            .parameters
            .get("scout_maximum_evidence_age_seconds")
            .copied()
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(|seconds| (seconds.min(u64::MAX as f64 / 1_000.0) as u64) * 1_000)
            .unwrap_or(DEFAULT_MAX_EVIDENCE_AGE_MS)
    };
    // Seed active profiles even when no Scout outcome has ever been recorded.
    // Otherwise an empty cohort is indistinguishable from a cohort that does
    // not need coverage, and downstream workers cannot prioritize it.
    for profile in state.active_profiles.values() {
        if !walk_forward_identity_allowed(&profile.symbol, &profile.strategy) {
            continue;
        }
        let cohort = format!("{}|{}", profile.symbol, profile.strategy);
        cohorts.entry(cohort.clone()).or_default();
        max_age_ms_by_cohort.insert(cohort, profile_max_age_ms(profile));
        let exact_profile = format!(
            "{}|{}|profile:{}",
            profile.symbol, profile.strategy, profile.profile_id
        );
        cohorts.entry(exact_profile.clone()).or_default();
        max_age_ms_by_cohort.insert(exact_profile, profile_max_age_ms(profile));
    }
    // Preserve queued direction-specific acquisition as an explicit gap even
    // before its first paired outcome arrives.
    for request in state.evidence_requests.values() {
        let Some(direction) = request
            .direction_scope
            .as_deref()
            .filter(|direction| matches!(*direction, "long" | "short"))
        else {
            continue;
        };
        if !walk_forward_identity_allowed(&request.symbol, &request.strategy) {
            continue;
        }
        let key = format!(
            "{}|{}|profile:{}|regime:{}|horizon:{}|direction:{}",
            request.symbol,
            request.strategy,
            request.profile_id,
            request.regime,
            request.horizon,
            direction,
        );
        cohorts.entry(key.clone()).or_default();
        let matching_profile = state
            .active_profiles
            .values()
            .find(|profile| {
                profile.symbol == request.symbol
                    && profile.strategy == request.strategy
                    && profile.profile_id == request.profile_id
            })
            .or_else(|| {
                state.variants.values().find_map(|variant| {
                    (variant.profile.symbol == request.symbol
                        && variant.profile.strategy == request.strategy
                        && variant.profile.profile_id == request.profile_id)
                        .then_some(&variant.profile)
                })
            });
        if let Some(profile) = matching_profile {
            max_age_ms_by_cohort.insert(key, profile_max_age_ms(profile));
        }
    }
    for variant in state.variants.values() {
        let symbol = &variant.profile.symbol;
        let strategy_name = &variant.profile.strategy;
        if !walk_forward_identity_allowed(symbol, strategy_name) {
            continue;
        }
        let cohort = format!("{symbol}|{strategy_name}");
        let exact_profile = format!(
            "{symbol}|{strategy_name}|profile:{}",
            variant.profile.profile_id
        );
        let max_age_ms = profile_max_age_ms(&variant.profile);
        max_age_ms_by_cohort
            .entry(cohort.clone())
            .or_insert(max_age_ms);
        max_age_ms_by_cohort.insert(exact_profile.clone(), max_age_ms);
        cohorts.entry(exact_profile.clone()).or_default();
        for outcome in &variant.outcomes {
            if outcome.source != LearningOutcomeSource::Scout
                || !outcome.net_return_percent.is_finite()
                || outcome.observed_at_ms >= now_ms
            {
                continue;
            }
            let day = outcome.observed_at_ms / 86_400_000;
            let regime = outcome
                .market_regime
                .as_deref()
                .filter(|value| matches!(*value, "range" | "trend" | "volatile" | "mixed"))
                .unwrap_or("unknown");
            let horizon = outcome
                .horizon_seconds
                .map(|seconds| seconds.to_string())
                .unwrap_or_else(|| "default".to_owned());
            let regime_cohort = format!("{cohort}|regime:{regime}");
            let regime_profile = format!("{exact_profile}|regime:{regime}");
            let horizon_cohort = format!("{cohort}|horizon:{horizon}");
            let horizon_profile = format!("{exact_profile}|horizon:{horizon}");
            let exact_case_cohort = format!("{cohort}|regime:{regime}|horizon:{horizon}");
            let exact_case_profile = format!("{exact_profile}|regime:{regime}|horizon:{horizon}");
            let mut keys = vec![
                cohort.clone(),
                exact_profile.clone(),
                regime_cohort,
                regime_profile,
                horizon_cohort,
                horizon_profile,
                exact_case_cohort,
                exact_case_profile,
            ];
            if let Some(direction) = outcome
                .learning_context
                .as_deref()
                .and_then(|context| scout_context_direction(context, symbol, strategy_name, regime))
            {
                keys.push(format!(
                    "{exact_profile}|regime:{regime}|horizon:{horizon}|direction:{direction}"
                ));
            }
            for key in &keys {
                max_age_ms_by_cohort
                    .entry(key.clone())
                    .or_insert(max_age_ms);
            }
            for key in keys {
                if let Some(group) = outcome.evidence_group_id.as_deref()
                    && !seen.insert(format!("{key}|{group}"))
                {
                    continue;
                }
                *counts.entry(key.clone()).or_default() += 1;
                last_samples
                    .entry(key.clone())
                    .and_modify(|latest| *latest = (*latest).max(outcome.observed_at_ms))
                    .or_insert(outcome.observed_at_ms);
                *cohorts
                    .entry(key.clone())
                    .or_default()
                    .entry(day)
                    .or_default() += outcome_decay_weight(outcome.observed_at_ms, now_ms);
            }
        }
    }
    cohorts
        .into_iter()
        .map(|(key, days)| {
            let sample_count = counts.get(&key).copied().unwrap_or_default();
            let weight_sum = days.values().sum::<f64>();
            let squared_weight_sum = days.values().map(|weight| weight * weight).sum::<f64>();
            let effective_sample_size = if squared_weight_sum > 0.0 {
                weight_sum * weight_sum / squared_weight_sum
            } else {
                0.0
            };
            let last_sample_timestamp_ms = last_samples.get(&key).copied().unwrap_or_default();
            let freshness_status = if sample_count == 0 {
                "missing"
            } else if now_ms.saturating_sub(last_sample_timestamp_ms)
                > max_age_ms_by_cohort
                    .get(&key)
                    .copied()
                    .unwrap_or(DEFAULT_MAX_EVIDENCE_AGE_MS)
            {
                "stale"
            } else {
                "fresh"
            };
            (
                key,
                serde_json::json!({
                    "sampleCount": sample_count,
                    "effectiveSampleSize": effective_sample_size,
                    "lastSampleTimestampMs": last_sample_timestamp_ms,
                    "freshnessStatus": freshness_status,
                }),
            )
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EvidenceGainCoverageSnapshot {
    pub(super) status: String,
    pub(super) sample_count: Option<usize>,
    pub(super) effective_sample_size: Option<f64>,
    pub(super) last_sample_timestamp_ms: Option<u64>,
}

pub(super) fn evidence_gain_coverage_snapshot(
    state: &LearningState,
    cohort_id: Option<&str>,
    now_ms: u64,
) -> EvidenceGainCoverageSnapshot {
    let coverage =
        cohort_id.and_then(|key| walk_forward_priority_snapshot(state, now_ms).remove(key));
    EvidenceGainCoverageSnapshot {
        status: coverage
            .as_ref()
            .and_then(|value| value["freshnessStatus"].as_str())
            .unwrap_or("unknown")
            .to_owned(),
        sample_count: coverage
            .as_ref()
            .and_then(|value| value["sampleCount"].as_u64())
            .map(|value| value as usize),
        effective_sample_size: coverage
            .as_ref()
            .and_then(|value| value["effectiveSampleSize"].as_f64())
            .filter(|value| value.is_finite() && *value >= 0.0),
        last_sample_timestamp_ms: coverage
            .as_ref()
            .and_then(|value| value["lastSampleTimestampMs"].as_u64())
            .filter(|timestamp| *timestamp > 0),
    }
}

/// Links a request completion only to the unique event that observed the
/// exact request cohort transition from missing/stale to fresh in this attempt.
/// It is telemetry only and does not alter evidence or promotion calculations.
pub(super) fn record_evidence_gain_request_completion(
    state: &mut LearningState,
    request_id: &str,
    now_ms: u64,
) -> bool {
    let Some(request) = state.evidence_requests.get(request_id).cloned() else {
        return false;
    };
    if request.status != EvidenceRequestStatus::Completed
        || request.completion_source_event_id.is_some()
    {
        return false;
    }
    let cohort_id = evidence_request_cohort_id(&request);
    let matching = state
        .evidence_gain_records
        .iter()
        .filter(|(_, record)| {
            record.evidence_request_id.as_deref() == Some(request_id)
                && record.evidence_request_attempt == Some(request.attempts)
                && record.evidence_cohort_id.as_deref() == Some(cohort_id.as_str())
                && record.new_coverage_observed
                && record.coverage_after_status == "fresh"
                && record.request_closed_at_ms.is_none()
        })
        .map(|(record_id, _)| record_id.clone())
        .collect::<Vec<_>>();
    let [record_id] = matching.as_slice() else {
        return false;
    };
    let Some(record) = state.evidence_gain_records.get_mut(record_id) else {
        return false;
    };
    record.request_closed_at_ms = Some(now_ms);
    if record.source_event_id.is_some() {
        if let Some(request) = state.evidence_requests.get_mut(request_id) {
            request.completion_source_event_id = record.source_event_id.clone();
            request.completion_trade_id = record.trade_id.clone();
            request.completion_episode_id = record.episode_id.clone();
        }
    }
    true
}

pub(super) fn evidence_request_cohort_id(request: &EvidenceRequest) -> String {
    match request.direction_scope.as_deref() {
        Some(direction) => format!(
            "{}|{}|profile:{}|regime:{}|horizon:{}|direction:{direction}",
            request.symbol, request.strategy, request.profile_id, request.regime, request.horizon
        ),
        None => format!(
            "{}|{}|profile:{}|regime:{}|horizon:{}",
            request.symbol, request.strategy, request.profile_id, request.regime, request.horizon
        ),
    }
}

fn decayed_profile(
    profile: &StrategyProfile,
    state: &LearningState,
    now_ms: u64,
) -> StrategyProfile {
    let Some(variant) = state.variants.values().find(|variant| {
        variant.profile.symbol == profile.symbol
            && variant.profile.strategy == profile.strategy
            && variant.profile.parameters == profile.parameters
    }) else {
        return profile.clone();
    };
    let last_update = variant
        .outcomes
        .iter()
        .map(|outcome| outcome.observed_at_ms)
        .max()
        .unwrap_or_default();
    if last_update == 0 {
        return profile.clone();
    }
    let weight =
        0.5_f64.powf(now_ms.saturating_sub(last_update) as f64 / PARAMETER_HALF_LIFE_MS as f64);
    let mut decayed = profile.clone();
    for spec in &variant.specs {
        if let Some(value) = decayed.parameters.get_mut(&spec.key) {
            *value = (spec.default + (*value - spec.default) * weight).clamp(spec.min, spec.max);
        }
    }
    decayed
}

pub(super) async fn get_profiles(
    State(service): State<ServiceState>,
) -> Json<Vec<StrategyProfile>> {
    let state = service.state.lock().await;
    let now = now_ms();
    Json(
        state
            .active_profiles
            .values()
            .map(|profile| decayed_profile(profile, &state, now))
            .collect(),
    )
}

pub(super) async fn get_decision_snapshot(
    State(service): State<ServiceState>,
) -> Json<LearningDecisionSnapshotResponse> {
    let snapshot_now_ms = now_ms();
    let state = service.state.lock().await;
    let profiles = state
        .active_profiles
        .values()
        .map(|profile| decayed_profile(profile, &state, snapshot_now_ms))
        .collect::<Vec<_>>();
    let registered_profile_ids = state
        .variants
        .values()
        .map(|variant| variant.profile.profile_id.clone())
        .collect::<BTreeSet<_>>();
    // Exact profile buckets remain regime-specific when a Scout carries a
    // regime. Unconditioned evidence is retained only for legacy observations.
    // Same-symbol aggregates remain available as a deliberate fallback; no
    // evidence is ever pooled across symbols or strategies.
    let mut grouped = BTreeMap::<String, (f64, f64, f64, u64)>::new();
    let mut risk_samples = BTreeMap::<String, Vec<WalkForwardSample>>::new();
    let mut seen_evidence = BTreeSet::<String>::new();
    for variant in state.variants.values() {
        if !walk_forward_identity_allowed(&variant.profile.symbol, &variant.profile.strategy) {
            continue;
        }
        for outcome in &variant.outcomes {
            let Some(regime) = outcome
                .market_regime
                .as_deref()
                .filter(|value| matches!(*value, "range" | "trend" | "volatile" | "mixed"))
            else {
                if !include_walk_forward_evidence(
                    outcome,
                    &variant.profile.profile_id,
                    snapshot_now_ms,
                    &mut seen_evidence,
                ) {
                    continue;
                }
                accumulate_walk_forward(
                    &mut grouped,
                    &variant.profile.profile_id,
                    outcome,
                    snapshot_now_ms,
                );
                accumulate_risk_sample(
                    &mut risk_samples,
                    &variant.profile.profile_id,
                    outcome,
                    snapshot_now_ms,
                );
                accumulate_walk_forward(
                    &mut grouped,
                    &format!("{}|{}", variant.profile.symbol, variant.profile.strategy),
                    outcome,
                    snapshot_now_ms,
                );
                accumulate_risk_sample(
                    &mut risk_samples,
                    &format!("{}|{}", variant.profile.symbol, variant.profile.strategy),
                    outcome,
                    snapshot_now_ms,
                );
                continue;
            };
            let exact_key = format!("{}|regime:{regime}", variant.profile.profile_id);
            if !include_walk_forward_evidence(
                outcome,
                &exact_key,
                snapshot_now_ms,
                &mut seen_evidence,
            ) {
                continue;
            }
            accumulate_walk_forward(&mut grouped, &exact_key, outcome, snapshot_now_ms);
            accumulate_risk_sample(&mut risk_samples, &exact_key, outcome, snapshot_now_ms);
            accumulate_walk_forward(
                &mut grouped,
                &format!(
                    "{}|{}|regime:{regime}",
                    variant.profile.symbol, variant.profile.strategy
                ),
                outcome,
                snapshot_now_ms,
            );
            accumulate_risk_sample(
                &mut risk_samples,
                &format!(
                    "{}|{}|regime:{regime}",
                    variant.profile.symbol, variant.profile.strategy
                ),
                outcome,
                snapshot_now_ms,
            );
            accumulate_walk_forward(
                &mut grouped,
                &format!("{}|{}", variant.profile.symbol, variant.profile.strategy),
                outcome,
                snapshot_now_ms,
            );
            accumulate_risk_sample(
                &mut risk_samples,
                &format!("{}|{}", variant.profile.symbol, variant.profile.strategy),
                outcome,
                snapshot_now_ms,
            );
        }
    }
    let mut walk_forward_evidence = BTreeMap::new();
    for (key, (weight, positive, net_return, last_sample_timestamp_ms)) in grouped {
        if weight > 0.0 {
            let risk = walk_forward_risk(risk_samples.remove(&key).unwrap_or_default());
            let profile_key = key
                .split_once("|regime:")
                .map_or(key.as_str(), |(profile, _)| profile);
            let regime_scoped = key.contains("|regime:");
            let profile_exact = registered_profile_ids.contains(profile_key);
            let evidence_scope = match (profile_exact, regime_scoped) {
                (true, true) => strategy::StrategyWalkForwardEvidenceScope::ExactProfileRegime,
                (true, false) => {
                    strategy::StrategyWalkForwardEvidenceScope::ExactProfileUnconditioned
                }
                (false, true) => {
                    strategy::StrategyWalkForwardEvidenceScope::SymbolStrategyRegimeFallback
                }
                (false, false) => {
                    strategy::StrategyWalkForwardEvidenceScope::SymbolStrategyUnconditionedFallback
                }
            };
            walk_forward_evidence.insert(
                key,
                StrategyWalkForwardEvidence {
                    evidence_contract_version: 2,
                    estimator: strategy::StrategyWalkForwardEstimator::CompletedOutcomeClusters,
                    history_coverage: strategy::StrategyWalkForwardCoverage::Partial,
                    cost_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
                    search_family_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
                    point_in_time_cutoff_timestamp_ms: Some(snapshot_now_ms),
                    final_holdout_status:
                        strategy::StrategyWalkForwardValidationStatus::NotValidated,
                    oos_fold_count: 0,
                    spa_p_value: None,
                    sample_count: risk.sample_count,
                    positive_net_rate: Some(positive / weight),
                    average_net_return_percent: Some(net_return / weight),
                    last_sample_timestamp_ms,
                    expectancy_ci_low_percent: risk.expectancy_ci_low_percent,
                    expectancy_ci_high_percent: risk.expectancy_ci_high_percent,
                    effective_sample_size: Some(risk.effective_sample_size),
                    window_count: risk.window_count,
                    max_drawdown_percent: risk.max_drawdown_percent,
                    downside_loss_percent: risk.downside_loss_percent,
                    evidence_scope,
                },
            );
        }
    }
    let cutoff = snapshot_now_ms.saturating_sub(7 * 24 * 60 * 60 * 1_000);
    let mut totals = BTreeMap::<String, Vec<(u64, f64)>>::new();
    let mut recent_profile_context_returns = BTreeMap::<String, Vec<(u64, f64)>>::new();
    for variant in state.variants.values() {
        let returns = totals.entry(variant.profile.strategy.clone()).or_default();
        for outcome in &variant.outcomes {
            if outcome.source != LearningOutcomeSource::Trade
                || !outcome.net_return_percent.is_finite()
            {
                continue;
            }
            returns.push((outcome.observed_at_ms, outcome.net_return_percent));
            if outcome.observed_at_ms < cutoff {
                continue;
            }
            let Some(context) = outcome.learning_context.as_deref() else {
                continue;
            };
            recent_profile_context_returns
                .entry(format!("{}|{}", variant.profile.profile_id, context))
                .or_default()
                .push((outcome.observed_at_ms, outcome.net_return_percent));
        }
    }
    for returns in recent_profile_context_returns.values_mut() {
        returns.sort_unstable_by_key(|sample| std::cmp::Reverse(sample.0));
        // Keep enough recent realized trades for the entry risk controller to
        // distinguish a persistent losing cohort from a short streak.
        returns.truncate(32);
    }
    let contextual_stop_loss_percents = contextual_stop_loss_percents(&state, cutoff);
    let mut exit_protection_policies = Vec::new();
    let mut exit_policy_assessments = Vec::new();
    for profile in &profiles {
        let Some(evidence) = crate::exit_policy::select_exit_policy_for_profile(
            state.exit_timing_observations.values(),
            &profile.symbol,
            &profile.strategy,
            &profile.profile_id,
            profile.version,
            snapshot_now_ms,
        ) else {
            exit_policy_assessments.push(
                crate::exit_policy::unavailable_exit_policy_assessment_for_profile(
                    state.exit_timing_observations.values(),
                    &profile.symbol,
                    &profile.strategy,
                    &profile.profile_id,
                    profile.version,
                    snapshot_now_ms,
                ),
            );
            continue;
        };
        exit_policy_assessments.push(strategy::LearnedExitPolicyAssessment {
            symbol: profile.symbol.clone(),
            strategy: profile.strategy.clone(),
            profile_id: Some(profile.profile_id.clone()),
            profile_version: Some(profile.version),
            stop_loss_percent: Some(evidence.stop_loss_percent),
            take_profit_percent: Some(evidence.take_profit_percent),
            activation_percent: Some(evidence.parameters.activation_percent),
            giveback_percent: Some(evidence.parameters.giveback_percent),
            minimum_lock_percent: Some(evidence.parameters.minimum_lock_percent),
            trailing_activation_percent: evidence.parameters.trailing_activation_percent,
            trailing_distance_percent: evidence.parameters.trailing_distance_percent,
            breakeven_activation_percent: evidence.parameters.breakeven_activation_percent,
            maximum_holding_duration_ms: evidence.maximum_holding_duration_ms,
            sample_count: evidence.sample_count,
            training_count: evidence.training_count,
            validation_count: evidence.validation_count,
            as_of_ms: evidence.as_of_ms,
            status: if evidence.metrics.promotion_eligible {
                "promotion_eligible".to_owned()
            } else if evidence.metrics.promotion_blockers.iter().any(|blocker| {
                blocker.starts_with("training_") || blocker.starts_with("validation_")
            }) {
                "candidate_rejected".to_owned()
            } else {
                "metrics_blocked".to_owned()
            },
            metrics: evidence.metrics.clone(),
        });
        if !evidence.metrics.promotion_eligible {
            continue;
        }
        exit_protection_policies.push(strategy::LearnedExitProtectionPolicy {
            symbol: profile.symbol.clone(),
            strategy: profile.strategy.clone(),
            profile_id: Some(profile.profile_id.clone()),
            profile_version: Some(profile.version),
            stop_loss_percent: Some(evidence.stop_loss_percent),
            take_profit_percent: Some(evidence.take_profit_percent),
            maximum_holding_duration_ms: evidence.maximum_holding_duration_ms,
            trailing_activation_percent: evidence.parameters.trailing_activation_percent,
            trailing_distance_percent: evidence.parameters.trailing_distance_percent,
            breakeven_activation_percent: evidence.parameters.breakeven_activation_percent,
            activation_percent: evidence.parameters.activation_percent,
            giveback_percent: evidence.parameters.giveback_percent,
            minimum_lock_percent: evidence.parameters.minimum_lock_percent,
            sample_count: evidence.sample_count,
            training_count: evidence.training_count,
            validation_count: evidence.validation_count,
            baseline_validation_mean_percent: evidence.baseline_validation_mean_percent,
            selected_validation_mean_percent: evidence.selected_validation_mean_percent,
            validation_gain_percent: evidence.validation_gain_percent,
            validation_positive_rate: evidence.validation_positive_rate,
            validation_negative_difference_rate: evidence.validation_negative_difference_rate,
            validation_tenth_percentile_difference_percent: evidence
                .validation_tenth_percentile_difference_percent,
            validation_worst_difference_percent: evidence.validation_worst_difference_percent,
            as_of_ms: evidence.as_of_ms,
        });
    }
    let strategy_size_multipliers = totals
        .into_iter()
        .map(|(strategy, returns)| {
            let multiplier =
                snapshot_strategy_size_multiplier(&strategy, &returns, snapshot_now_ms);
            (strategy, multiplier)
        })
        .collect();
    let snapshot = LearnedProfileSnapshot {
        profiles,
        registered_profile_ids,
        strategy_size_multipliers,
        recent_profile_context_returns,
        contextual_stop_loss_percents,
        context_learning: state.context_learning.clone(),
        context_thresholds: context::snapshot_context_thresholds(&state, now_ms()),
        missed_opportunity_bands: state
            .missed_opportunity_bands
            .iter()
            .take(2_000)
            .cloned()
            .collect(),
        execution_learning: state.execution_learning.clone(),
        walk_forward_evidence,
        scout_parameter_variants: scout_parameter_variant_snapshots(&state),
        exit_protection_policies,
        exit_policy_assessments,
    };
    Json(LearningDecisionSnapshotResponse {
        snapshot,
        replay_readiness: replay_readiness_summary(&state, snapshot_now_ms),
    })
}

#[derive(Serialize)]
pub(super) struct LearningDecisionSnapshotResponse {
    #[serde(flatten)]
    pub(super) snapshot: LearnedProfileSnapshot,
    #[serde(rename = "replayReadiness")]
    replay_readiness: ReplayReadinessSummary,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplayReadinessSummary {
    persisted_state_available: bool,
    scout_replay_runs: PersistedRunSummary,
    strategy_replay_jobs: ReplayJobSummary,
    missed_opportunity_jobs: ReplayJobSummary,
    retention: ReplayRetentionSummary,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRunSummary {
    run_count: usize,
    counts_by_status: BTreeMap<String, usize>,
    latest_created_at_ms: Option<u64>,
    latest_age_ms: Option<u64>,
    timestamped_run_count: usize,
    at_retention_limit: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplayJobSummary {
    job_count: usize,
    counts_by_status: BTreeMap<String, usize>,
    timestamp_source: String,
    oldest_timestamp_ms: Option<u64>,
    latest_timestamp_ms: Option<u64>,
    latest_age_ms: Option<u64>,
    timestamped_job_count: usize,
    at_retention_limit: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplayRetentionSummary {
    outcome_history_truncated: bool,
    seen_event_ids_overflowed: bool,
    seen_execution_event_ids_overflowed: bool,
    scout_replay_runs_at_limit: bool,
    strategy_replay_jobs_at_limit: bool,
    missed_opportunity_jobs_at_limit: bool,
}

fn status_counts(
    statuses: impl IntoIterator<Item = String>,
    known: &[&str],
) -> BTreeMap<String, usize> {
    let mut counts = known
        .iter()
        .map(|status| ((*status).to_owned(), 0))
        .collect::<BTreeMap<_, _>>();
    for status in statuses {
        *counts.entry(status).or_default() += 1;
    }
    counts
}

fn timestamp_summary(
    timestamps: impl Iterator<Item = u64>,
    now_ms: u64,
    at_limit: bool,
) -> (Option<u64>, Option<u64>, Option<u64>, usize, bool) {
    let values = timestamps
        .filter(|timestamp| *timestamp > 0)
        .collect::<Vec<_>>();
    let oldest = values.iter().copied().min();
    let latest = values.iter().copied().max();
    (
        oldest,
        latest,
        latest.map(|timestamp| now_ms.saturating_sub(timestamp)),
        values.len(),
        at_limit,
    )
}

fn replay_readiness_summary(state: &LearningState, now_ms: u64) -> ReplayReadinessSummary {
    const SCOUT_REPLAY_LIMIT: usize = 32;
    const STRATEGY_REPLAY_LIMIT: usize = 10_000;
    const MISSED_OPPORTUNITY_LIMIT: usize = 10_000;
    let scout_runs = state.scout_replay_runs.values().collect::<Vec<_>>();
    let scout_counts = status_counts(
        scout_runs
            .iter()
            .map(|run| run["status"].as_str().unwrap_or("unknown").to_owned()),
        &["pending", "running", "completed", "failed", "unknown"],
    );
    let scout_timestamps = scout_runs
        .iter()
        .filter_map(|run| run["createdAtMs"].as_u64());
    let (_oldest_scout, latest_scout, latest_scout_age, timestamped_scout_count, scout_at_limit) =
        timestamp_summary(
            scout_timestamps,
            now_ms,
            scout_runs.len() >= SCOUT_REPLAY_LIMIT,
        );
    let strategy_counts = status_counts(
        state
            .strategy_replay_jobs
            .values()
            .map(|job| format!("{:?}", job.status).to_lowercase()),
        &[
            "pending",
            "running",
            "completed",
            "failed",
            "unsupported",
            "unknown",
        ],
    );
    let (
        oldest_strategy,
        latest_strategy,
        latest_strategy_age,
        timestamped_strategy_count,
        strategy_at_limit,
    ) = timestamp_summary(
        state
            .strategy_replay_jobs
            .values()
            .map(|job| job.updated_at_ms),
        now_ms,
        state.strategy_replay_jobs.len() >= STRATEGY_REPLAY_LIMIT,
    );
    let missed_counts = status_counts(
        state
            .missed_opportunity_jobs
            .values()
            .map(|job| format!("{:?}", job.status).to_lowercase()),
        &[
            "pending",
            "running",
            "completed",
            "failed",
            "unsupported",
            "unknown",
        ],
    );
    let (
        oldest_missed,
        latest_missed,
        latest_missed_age,
        timestamped_missed_count,
        missed_at_limit,
    ) = timestamp_summary(
        state
            .missed_opportunity_jobs
            .values()
            .map(|job| job.observed_at_ms),
        now_ms,
        state.missed_opportunity_jobs.len() >= MISSED_OPPORTUNITY_LIMIT,
    );
    ReplayReadinessSummary {
        persisted_state_available: true,
        scout_replay_runs: PersistedRunSummary {
            run_count: scout_runs.len(),
            counts_by_status: scout_counts,
            latest_created_at_ms: latest_scout,
            latest_age_ms: latest_scout_age,
            timestamped_run_count: timestamped_scout_count,
            at_retention_limit: scout_at_limit,
        },
        strategy_replay_jobs: ReplayJobSummary {
            job_count: state.strategy_replay_jobs.len(),
            counts_by_status: strategy_counts,
            timestamp_source: "updated_at_ms".to_owned(),
            oldest_timestamp_ms: oldest_strategy,
            latest_timestamp_ms: latest_strategy,
            latest_age_ms: latest_strategy_age,
            timestamped_job_count: timestamped_strategy_count,
            at_retention_limit: strategy_at_limit,
        },
        missed_opportunity_jobs: ReplayJobSummary {
            job_count: state.missed_opportunity_jobs.len(),
            counts_by_status: missed_counts,
            timestamp_source: "observed_at_ms".to_owned(),
            oldest_timestamp_ms: oldest_missed,
            latest_timestamp_ms: latest_missed,
            latest_age_ms: latest_missed_age,
            timestamped_job_count: timestamped_missed_count,
            at_retention_limit: missed_at_limit,
        },
        retention: ReplayRetentionSummary {
            outcome_history_truncated: state.outcome_history_truncated,
            seen_event_ids_overflowed: state.seen_event_ids_overflowed,
            seen_execution_event_ids_overflowed: state.seen_execution_event_ids_overflowed,
            scout_replay_runs_at_limit: scout_at_limit,
            strategy_replay_jobs_at_limit: strategy_at_limit,
            missed_opportunity_jobs_at_limit: missed_at_limit,
        },
    }
}

fn walk_forward_identity_allowed(symbol: &str, strategy_name: &str) -> bool {
    strategy::is_autotrader_symbol_allowed(symbol)
        && strategy::Strategy::from_name(strategy_name)
            .is_some_and(|strategy| !strategy.is_legacy_composite())
}

fn include_walk_forward_evidence(
    outcome: &LearningOutcome,
    cohort: &str,
    candidate_at_ms: u64,
    seen_evidence: &mut BTreeSet<String>,
) -> bool {
    if outcome.source != LearningOutcomeSource::Scout
        || !outcome.net_return_percent.is_finite()
        // Strict point-in-time cutoff: evidence evaluated at the candidate
        // timestamp is not yet prior evidence.
        || outcome.observed_at_ms >= candidate_at_ms
    {
        return false;
    }
    outcome
        .evidence_group_id
        .as_deref()
        .is_none_or(|group_id| seen_evidence.insert(format!("{cohort}|{group_id}")))
}

fn accumulate_walk_forward(
    grouped: &mut BTreeMap<String, (f64, f64, f64, u64)>,
    key: &str,
    outcome: &LearningOutcome,
    now_ms: u64,
) {
    let weight = outcome_decay_weight(outcome.observed_at_ms, now_ms);
    let bucket = grouped.entry(key.to_owned()).or_default();
    bucket.0 += weight;
    bucket.1 += if outcome.net_return_percent > 0.0 {
        weight
    } else {
        0.0
    };
    bucket.2 += weight * outcome.net_return_percent;
    bucket.3 = bucket.3.max(outcome.observed_at_ms);
}

fn accumulate_risk_sample(
    samples: &mut BTreeMap<String, Vec<WalkForwardSample>>,
    key: &str,
    outcome: &LearningOutcome,
    now_ms: u64,
) {
    samples
        .entry(key.to_owned())
        .or_default()
        .push(WalkForwardSample {
            observed_at_ms: outcome.observed_at_ms,
            net_return_percent: outcome.net_return_percent,
            weight: outcome_decay_weight(outcome.observed_at_ms, now_ms),
        });
}

fn walk_forward_risk(samples: Vec<WalkForwardSample>) -> WalkForwardStatistics {
    summarize_walk_forward_samples(samples)
}

const MIN_CONTEXTUAL_STOP_REPLAYS: usize = 5;

//TODO comment this fucntion
fn contextual_stop_loss_percents(state: &LearningState, cutoff: u64) -> BTreeMap<String, f64> {
    let mut candidates = BTreeMap::<String, BTreeMap<String, Vec<f64>>>::new();
    for observation in state.exit_timing_observations.values() {
        if observation["reason"].as_str() != Some("stop-loss")
            || observation["observed_at_ms"].as_u64().unwrap_or_default() < cutoff
        {
            continue;
        }
        let entry = &observation["entry_context"];
        let Some(profile_id) = entry["strategy_decision"]["learning_profile_id"].as_str() else {
            continue;
        };
        let Some(context) =
            market_context_from_learning_context(observation["learning_context"].as_str())
        else {
            continue;
        };
        let Some(current_stop) = observation["effective_stop_loss_percent"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
        else {
            continue;
        };
        let Some(entry_price) = entry["average_price"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
        else {
            continue;
        };
        let Some(current_return) = observation["exit_return_percent"]
            .as_f64()
            .filter(|v| v.is_finite())
        else {
            continue;
        };
        let Some(strategy) = observation["strategy"]
            .as_str()
            .and_then(strategy::Strategy::from_name)
        else {
            continue;
        };
        let specs = strategy.parameter_specs();
        let Some(spec) = specs
            .iter()
            .find(|spec| spec.key.as_str() == "stop_loss_percent")
        else {
            continue;
        };
        let candidate_stop = (current_stop + spec.step).min(spec.max);
        if candidate_stop <= current_stop + 1e-9 {
            continue;
        }
        let long = observation["side"].as_str() == Some("sell");
        let mut path = observation["pre_exit_price_path"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        path.extend(
            observation["price_path"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        path.sort_unstable_by_key(|sample| sample["timestamp"].as_u64().unwrap_or_default());
        if path.len() < 3 {
            continue;
        }
        let threshold = if long {
            entry_price * (1.0 - candidate_stop / 100.0)
        } else {
            entry_price * (1.0 + candidate_stop / 100.0)
        };
        let exit = path
            .iter()
            .find_map(|sample| {
                let executable = if long {
                    sample["bid"].as_f64()?
                } else {
                    sample["ask"].as_f64()?
                };
                ((long && executable <= threshold) || (!long && executable >= threshold))
                    .then_some(executable)
            })
            .or_else(|| {
                path.last().and_then(|sample| {
                    if long {
                        sample["bid"].as_f64()
                    } else {
                        sample["ask"].as_f64()
                    }
                })
            });
        let Some(exit) = exit.filter(|v| v.is_finite() && *v > 0.0) else {
            continue;
        };
        let gross = if long {
            exit / entry_price - 1.0
        } else {
            entry_price / exit - 1.0
        } * 100.0;
        let quantity = entry["quantity"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(0.0);
        let fees = entry["fees"].as_f64().unwrap_or(0.0).abs()
            + observation["exit_fees"].as_f64().unwrap_or(0.0).abs();
        let replay_return = gross
            - if quantity > 0.0 {
                fees / (entry_price * quantity) * 100.0
            } else {
                0.0
            };
        if !replay_return.is_finite() {
            continue;
        }
        candidates
            .entry(format!("{profile_id}|{context}"))
            .or_default()
            .entry(format!("{candidate_stop:.8}"))
            .or_default()
            .push(replay_return - current_return);
    }
    candidates
        .into_iter()
        .filter_map(|(key, values)| {
            values
                .into_iter()
                .filter_map(|(candidate, improvements)| {
                    if improvements.len() < MIN_CONTEXTUAL_STOP_REPLAYS {
                        return None;
                    }
                    candidate.parse::<f64>().ok().map(|candidate| {
                        let mean = improvements.iter().sum::<f64>() / improvements.len() as f64;
                        let variance = improvements
                            .iter()
                            .map(|value| (value - mean).powi(2))
                            .sum::<f64>()
                            / improvements.len() as f64;
                        let lower =
                            mean - 1.645 * (variance.sqrt() / (improvements.len() as f64).sqrt());
                        (candidate, lower)
                    })
                })
                .filter(|(_, lower)| *lower > 0.0)
                .max_by(|(_, left), (_, right)| left.total_cmp(right))
                .map(|(candidate, _)| (key, candidate))
        })
        .collect()
}

fn market_context_from_learning_context(value: Option<&str>) -> Option<String> {
    let value = value?;
    let context = value.split(":P").next().unwrap_or(value);
    let mut parts = context.split(':');
    let (Some(side), Some(regime), Some(symbol), Some(strategy)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    (matches!(side, "long" | "short")
        && matches!(regime, "range" | "trend" | "volatile" | "mixed")
        && !symbol.is_empty()
        && !strategy.is_empty()
        && parts.next().is_none())
    .then(|| context.to_owned())
}

const MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE: usize = 20;

fn snapshot_strategy_size_multiplier(strategy: &str, returns: &[(u64, f64)], now_ms: u64) -> f64 {
    // Live strategies use contextual entry risk and validated Scout evidence.
    // A strategy-wide zero hides symbol/direction winners.
    if strategy::Strategy::from_name(strategy).is_some_and(|kind| kind.is_live_insight()) {
        1.0
    } else {
        strategy_size_multiplier(returns, now_ms)
    }
}

fn strategy_size_multiplier(net_returns: &[(u64, f64)], now_ms: u64) -> f64 {
    let (samples, positive, weighted_net_return) = net_returns.iter().fold(
        (0.0, 0.0, 0.0),
        |(samples, positive, net), (timestamp, return_percent)| {
            let weight = outcome_decay_weight(*timestamp, now_ms);
            (
                samples + weight,
                positive + if *return_percent > 0.0 { weight } else { 0.0 },
                net + weight * return_percent,
            )
        },
    );
    if samples >= MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE as f64 && weighted_net_return / samples <= 0.0
    {
        return 0.0;
    }
    if samples <= 0.0 || positive > samples {
        return 1.0;
    }
    let n = samples;
    let posterior = (positive as f64 + 1.0) / (n + 2.0);
    let confidence = n / (n + 2.0);
    let edge = ((posterior - 0.5) * 2.0 * confidence).clamp(-1.0, 1.0);
    let target = if edge < 0.0 {
        1.0 + edge * 0.30
    } else {
        1.0 + edge * 1.50
    };
    target.clamp(0.70, 2.50)
}

pub(super) fn rebuild_trade_profile_metrics(state: &mut LearningState, now: u64) {
    let cutoff = now.saturating_sub(7 * 24 * 60 * 60 * 1_000);
    let mut totals = BTreeMap::<String, (usize, usize, f64)>::new();
    for variant in state.variants.values_mut() {
        let mut samples = 0;
        let mut positive = 0;
        let mut realized_pnl = 0.0;
        for outcome in variant.outcomes.iter().filter(|outcome| {
            outcome.source == LearningOutcomeSource::Trade && outcome.observed_at_ms >= cutoff
        }) {
            samples += 1;
            positive += usize::from(outcome.net_return_percent > 0.0);
            realized_pnl += outcome.realized_pnl_usd.unwrap_or_default();
        }
        variant.samples = samples;
        variant.positive = positive;
        variant.realized_pnl = realized_pnl;
        let total = totals
            .entry(format!(
                "{}:{}",
                variant.profile.symbol, variant.profile.strategy
            ))
            .or_default();
        total.0 += samples;
        total.1 += positive;
        total.2 += realized_pnl;
    }
    for (key, profile) in &mut state.active_profiles {
        let (samples, positive, realized_pnl) = totals.get(key).copied().unwrap_or_default();
        if profile.learning_samples != samples
            || profile.positive_samples != positive
            || (profile.realized_pnl - realized_pnl).abs() > 1e-9
        {
            profile.learning_samples = samples;
            profile.positive_samples = positive;
            profile.realized_pnl = realized_pnl;
            profile.version = profile.version.saturating_add(1);
        }
    }
}

pub(super) fn scout_parameter_variant_snapshots(
    state: &LearningState,
) -> Vec<strategy::ScoutParameterVariant> {
    type CandidateKey = (String, String, String, String, Option<String>, String);
    let mut outcomes = BTreeMap::<CandidateKey, BTreeMap<String, (u64, Vec<f64>)>>::new();
    let mut candidate_families = BTreeMap::<CandidateKey, String>::new();
    let mut baseline_directions = BTreeMap::<CandidateKey, [usize; 4]>::new();
    let mut baseline_deltas = BTreeMap::<CandidateKey, (f64, usize)>::new();
    let mut direction_groups = BTreeSet::<(CandidateKey, String)>::new();
    let mut delta_groups = BTreeSet::<(CandidateKey, String)>::new();
    for variant in state.variants.values() {
        for outcome in &variant.outcomes {
            // Multi-parameter comparisons are promoted as a tested group, so
            // don't present their returns as isolated single-parameter tests.
            if outcome.tested_parameter_keys.len() != 1
                || !outcome.paired_comparison
                || outcome.independence_block_id.is_none()
                || outcome.candidate_family_id.is_none()
                || outcome.evidence_group_id.is_none()
            {
                continue;
            }
            let parameter = &outcome.tested_parameter_keys[0];
            let Some(value) = variant.profile.parameters.get(parameter) else {
                continue;
            };
            let key = (
                variant.profile.symbol.clone(),
                variant.profile.strategy.clone(),
                parameter.clone(),
                format!("{value:.8}"),
                outcome
                    .market_regime
                    .as_deref()
                    .filter(|regime| matches!(*regime, "range" | "trend" | "volatile" | "mixed"))
                    .map(str::to_owned),
                if outcome.baseline_parameters.is_empty() {
                    String::new()
                } else {
                    strategy::profile_id(
                        &variant.profile.symbol,
                        &variant.profile.strategy,
                        &outcome.baseline_parameters,
                    )
                },
            );
            let group_id = outcome.evidence_group_id.clone().unwrap();
            let block_id = outcome.independence_block_id.unwrap();
            let grouped = outcomes
                .entry(key.clone())
                .or_default()
                .entry(group_id.clone())
                .or_insert_with(|| (block_id, Vec::new()));
            grouped.0 = grouped.0.min(block_id);
            grouped.1.push(outcome.net_return_percent);
            candidate_families
                .entry(key.clone())
                .or_insert_with(|| outcome.candidate_family_id.clone().unwrap());
            let direction_counts = baseline_directions.entry(key.clone()).or_default();
            let baseline = outcome.baseline_parameters.get(parameter).copied();
            let step = variant
                .specs
                .iter()
                .find(|spec| spec.key == *parameter)
                .map_or(1e-8, |spec| spec.step * PARAMETER_STEP_RELATIVE_TOLERANCE);
            if direction_groups.insert((key.clone(), group_id.clone())) {
                match baseline {
                    Some(baseline) if *value > baseline + step => direction_counts[0] += 1,
                    Some(baseline) if *value < baseline - step => direction_counts[1] += 1,
                    Some(_) => direction_counts[2] += 1,
                    None => direction_counts[3] += 1,
                }
            }
            if let Some(baseline) =
                baseline.filter(|_| delta_groups.insert((key.clone(), group_id)))
            {
                let delta = baseline_deltas.entry(key.clone()).or_default();
                delta.0 += *value - baseline;
                delta.1 += 1;
            }
        }
    }

    let mut snapshots = Vec::new();
    for (key, grouped_comparisons) in outcomes {
        let (symbol, strategy, parameter, value_string, market_regime, baseline_profile_id) =
            key.clone();
        let Ok(candidate_value) = value_string.parse::<f64>() else {
            continue;
        };
        if grouped_comparisons.is_empty() {
            continue;
        }
        let mut day_comparisons = BTreeMap::<u64, Vec<f64>>::new();
        for (_, (day, observations)) in grouped_comparisons {
            if observations.iter().any(|value| !value.is_finite()) {
                continue;
            }
            day_comparisons
                .entry(day)
                .or_default()
                .push(observations.iter().sum::<f64>() / observations.len() as f64);
        }
        let blocks = day_comparisons
            .into_iter()
            .map(|(day, values)| (day, values.iter().sum::<f64>() / values.len() as f64))
            .collect::<Vec<_>>();
        let returns = blocks.iter().map(|(_, value)| *value).collect::<Vec<_>>();
        let sample_count = blocks.len();
        if sample_count == 0 {
            continue;
        }
        let direction_counts = baseline_directions.get(&key).copied().unwrap_or_default();
        let (paired_delta_sum, paired_delta_samples) =
            baseline_deltas.get(&key).copied().unwrap_or_default();
        let paired_delta_mean =
            (paired_delta_samples > 0).then_some(paired_delta_sum / paired_delta_samples as f64);
        let minimum_samples = state.minimum_live_scout_parameter_samples;
        let mean = returns.iter().sum::<f64>() / sample_count as f64;
        let family_id = candidate_families.get(&key).map(String::as_str);
        let history = family_id.and_then(|family| state.scout_promotion_look_history.get(family));
        let alpha = history
            .filter(|history| !history.overflowed)
            .and_then(|history| {
                super::feedback::scout_promotion_alpha(
                    0.1,
                    history.look_count,
                    history.family_hypothesis_count.unwrap_or_default(),
                )
            });
        let bootstrap_replicates = alpha.and_then(promotion_bootstrap_replicates);
        let (lower_bound, upper_bound) = if sample_count >= minimum_samples {
            bootstrap_replicates.map_or((None, None), |replicates| {
                let alpha = alpha.unwrap();
                let lower = stationary_bootstrap_lower_bound(
                    &blocks,
                    alpha,
                    MEAN_BLOCK_LENGTH_ONLINE_WINDOWS,
                    replicates,
                );
                let negated = blocks
                    .iter()
                    .map(|(day, value)| (*day, -*value))
                    .collect::<Vec<_>>();
                let upper = stationary_bootstrap_lower_bound(
                    &negated,
                    alpha,
                    MEAN_BLOCK_LENGTH_ONLINE_WINDOWS,
                    replicates,
                )
                .map(|bound| -bound);
                (lower, upper)
            })
        } else {
            (None, None)
        };
        let positive_evidence = lower_bound.is_some_and(|bound| bound > 0.0);
        let negative_evidence = upper_bound.is_some_and(|bound| bound < 0.0);
        let active_value = state
            .active_profiles
            .get(&format!("{symbol}:{strategy}"))
            .and_then(|profile| profile.parameters.get(&parameter))
            .copied();
        snapshots.push(strategy::ScoutParameterVariant {
            symbol,
            strategy,
            market_regime,
            baseline_profile_id,
            parameter,
            candidate_value,
            sample_count,
            minimum_samples,
            average_net_improvement_percent: mean,
            lower_90_one_sided_improvement_percent: lower_bound,
            upper_90_one_sided_improvement_percent: upper_bound,
            paired_baseline_increase_samples: direction_counts[0],
            paired_baseline_decrease_samples: direction_counts[1],
            paired_baseline_same_samples: direction_counts[2],
            paired_baseline_unknown_samples: direction_counts[3],
            paired_baseline_delta_mean: paired_delta_mean,
            paired_baseline_delta_samples: paired_delta_samples,
            promoted: positive_evidence
                && active_value.is_some_and(|value| (value - candidate_value).abs() <= 1e-8),
            rejected: negative_evidence,
        });
    }
    snapshots.sort_by(|left, right| {
        (&left.symbol, &left.strategy, &left.parameter)
            .cmp(&(&right.symbol, &right.strategy, &right.parameter))
            .then_with(|| left.candidate_value.total_cmp(&right.candidate_value))
    });
    snapshots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_state() -> LearningState {
        let mut state = LearningState::default();
        let strategy_job = |event_id: &str, status: &str, timestamp: u64| {
            serde_json::from_value(serde_json::json!({
                "event_id": event_id, "symbol": "BTC-USDT", "strategy": "live-momentum",
                "profile_id": "profile", "parameters": {}, "context": null,
                "status": status, "attempts": 0, "created_at_ms": timestamp,
                "updated_at_ms": timestamp, "reason": null, "result": null
            }))
            .unwrap()
        };
        for (id, status, timestamp) in [
            ("p", "pending", 100),
            ("c", "completed", 200),
            ("f", "failed", 300),
        ] {
            state
                .strategy_replay_jobs
                .insert(id.into(), strategy_job(id, status, timestamp));
            let missed: MissedOpportunityJob = serde_json::from_value(serde_json::json!({
                "event_id": id, "observed_at_ms": timestamp, "symbol": "BTC-USDT",
                "strategy": "live-momentum", "profile_id": "profile", "parameters": {},
                "direction": "long", "strength": 0.5, "score": 0.5,
                "learning_context": "long:trend:BTC-USDT", "status": status,
                "reason": null, "horizons": {}
            }))
            .unwrap();
            state.missed_opportunity_jobs.insert(id.into(), missed);
            state.scout_replay_runs.insert(
                format!("run-{id}"),
                serde_json::json!({
                    "runId": format!("run-{id}"), "status": status, "createdAtMs": timestamp
                }),
            );
        }
        state
    }

    #[test]
    fn sufficiently_sampled_negative_net_trades_pause_daily_strategy_sizing() {
        let now = now_ms();
        let losing = vec![(now, -0.01); MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE];
        assert_eq!(strategy_size_multiplier(&losing, now), 0.0);
        assert!(strategy_size_multiplier(&losing[..19], now) > 0.0);
        let winning = vec![(now, 0.01); MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE];
        assert!(strategy_size_multiplier(&winning, now) > 0.0);
        // A high win rate cannot hide negative average net returns.
        let mostly_winning = (0..MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE)
            .map(|index| (now, if index < 18 { 0.01 } else { -0.30 }))
            .collect::<Vec<_>>();
        assert_eq!(strategy_size_multiplier(&mostly_winning, now), 0.0);
    }

    #[tokio::test]
    async fn decision_snapshot_reports_empty_and_persisted_replay_work_without_changing_legacy_shape()
     {
        let empty_service = ServiceState {
            state: Arc::new(tokio::sync::Mutex::new(LearningState::default())),
            path: PathBuf::new(),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let Json(empty) = get_decision_snapshot(State(empty_service)).await;
        let empty_json = serde_json::to_value(empty).unwrap();
        assert!(
            empty_json.get("profiles").is_some(),
            "legacy snapshot fields stay at top level"
        );
        assert!(
            empty_json["replayReadiness"]["persistedStateAvailable"]
                .as_bool()
                .unwrap()
        );
        assert_eq!(
            empty_json["replayReadiness"]["scoutReplayRuns"]["runCount"],
            0
        );
        assert_eq!(
            empty_json["replayReadiness"]["strategyReplayJobs"]["countsByStatus"]["pending"],
            0
        );
        assert_eq!(
            empty_json["replayReadiness"]["missedOpportunityJobs"]["latestTimestampMs"],
            serde_json::Value::Null
        );

        let state = job_state();
        // The state structs and job maps use the same serde representation as persisted state.
        let restarted: LearningState =
            serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
        let service = ServiceState {
            state: Arc::new(tokio::sync::Mutex::new(restarted)),
            path: PathBuf::new(),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let Json(response) = get_decision_snapshot(State(service)).await;
        let value = serde_json::to_value(response).unwrap();
        let readiness = &value["replayReadiness"];
        assert_eq!(readiness["scoutReplayRuns"]["runCount"], 3);
        assert_eq!(readiness["scoutReplayRuns"]["countsByStatus"]["pending"], 1);
        assert_eq!(
            readiness["scoutReplayRuns"]["countsByStatus"]["completed"],
            1
        );
        assert_eq!(readiness["scoutReplayRuns"]["countsByStatus"]["failed"], 1);
        assert_eq!(readiness["strategyReplayJobs"]["jobCount"], 3);
        assert_eq!(
            readiness["strategyReplayJobs"]["countsByStatus"]["pending"],
            1
        );
        assert_eq!(
            readiness["missedOpportunityJobs"]["countsByStatus"]["completed"],
            1
        );
        assert_eq!(readiness["scoutReplayRuns"]["latestCreatedAtMs"], 300);
        assert!(
            readiness["scoutReplayRuns"]["latestAgeMs"]
                .as_u64()
                .is_some()
        );

        let missing_fields: LearningState = serde_json::from_value(serde_json::json!({
            "variants": {}, "active_profiles": {}, "seen_event_ids": [], "accepted_events": 0
        }))
        .unwrap();
        let missing_summary = replay_readiness_summary(&missing_fields, 500);
        assert_eq!(missing_summary.scout_replay_runs.run_count, 0);
        assert_eq!(missing_summary.strategy_replay_jobs.job_count, 0);
        assert!(missing_summary.persisted_state_available);

        let mut incomplete_run = LearningState::default();
        incomplete_run.scout_replay_runs.insert(
            "legacy-run".into(),
            serde_json::json!({"runId": "legacy-run"}),
        );
        let incomplete_summary = replay_readiness_summary(&incomplete_run, 500);
        assert_eq!(incomplete_summary.scout_replay_runs.run_count, 1);
        assert_eq!(
            incomplete_summary.scout_replay_runs.counts_by_status["unknown"],
            1
        );
        assert_eq!(
            incomplete_summary.scout_replay_runs.timestamped_run_count,
            0
        );
        assert_eq!(
            incomplete_summary.scout_replay_runs.latest_created_at_ms,
            None
        );
    }

    #[test]
    fn live_strategy_losses_do_not_create_strategy_wide_pause() {
        let now = now_ms();
        let losing = vec![(now, -0.01); MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE];
        assert_eq!(
            snapshot_strategy_size_multiplier("live-momentum", &losing, now),
            1.0
        );
        assert_eq!(
            snapshot_strategy_size_multiplier("donchian-20-btc200", &losing, now),
            0.0
        );
    }

    #[test]
    fn walk_forward_evidence_uses_strict_cutoff_and_deduplicates_groups() {
        let mut outcome = scout_variant("BTC-USDT", &[0.2]).outcomes.remove(0);
        outcome.evidence_group_id = Some("replay-1".into());
        let mut seen = BTreeSet::new();

        assert!(include_walk_forward_evidence(
            &outcome,
            "profile|regime:trend",
            10,
            &mut seen,
        ));
        assert!(!include_walk_forward_evidence(
            &outcome,
            "profile|regime:trend",
            10,
            &mut seen,
        ));
        assert!(include_walk_forward_evidence(
            &outcome,
            "another-profile|regime:trend",
            10,
            &mut seen,
        ));

        outcome.observed_at_ms = 10;
        assert!(!include_walk_forward_evidence(
            &outcome,
            "profile|regime:trend",
            10,
            &mut seen,
        ));
        outcome.observed_at_ms = 11;
        assert!(!include_walk_forward_evidence(
            &outcome,
            "profile|regime:trend",
            10,
            &mut seen,
        ));
    }

    #[test]
    fn scout_priority_snapshot_counts_independent_weighted_day_clusters() {
        let now = 10 * 86_400_000;
        let mut variant = scout_variant("BTC-USDT", &[0.1, 0.2, 0.3]);
        variant.profile.strategy = "live-momentum".into();
        variant.outcomes[0].observed_at_ms = now - 86_400_000 - 1;
        variant.outcomes[0].evidence_group_id = Some("same-replay".into());
        variant.outcomes[1].observed_at_ms = now - 86_400_000 - 2;
        variant.outcomes[1].evidence_group_id = Some("same-replay".into());
        variant.outcomes[2].observed_at_ms = now - 2 * 86_400_000 - 1;

        let mut state = LearningState::default();
        state.variants.insert("btc-profile".into(), variant);
        let snapshot = walk_forward_priority_snapshot(&state, now);
        let evidence = &snapshot["BTC-USDT|live-momentum"];
        assert_eq!(evidence["sampleCount"], 2);
        let effective_n = evidence["effectiveSampleSize"].as_f64().unwrap();
        assert!(effective_n > 1.0 && effective_n < 2.0);
    }

    #[test]
    fn negative_pause_eases_when_old_outcomes_decay_below_sample_threshold() {
        let now = now_ms();
        let one_half_life_ago = now.saturating_sub(OUTCOME_HALF_LIFE_MS);
        let losing = vec![(one_half_life_ago, -0.01); MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE];
        assert!(strategy_size_multiplier(&losing, now) > 0.0);
    }

    #[test]
    fn walk_forward_evidence_only_accepts_supported_symbols_and_strategies() {
        assert!(walk_forward_identity_allowed("BTC-USDT", "live-momentum"));
        assert!(!walk_forward_identity_allowed(
            "XMSTR-USDT",
            "live-momentum"
        ));
        assert!(!walk_forward_identity_allowed(
            "BTC-USDT",
            "unknown-strategy"
        ));
    }

    #[test]
    fn walk_forward_priority_distinguishes_missing_fresh_and_stale_cohorts() {
        let now = 3 * 86_400_000;
        let mut variant = scout_variant("BTC-USDT", &[0.1]);
        variant.profile.strategy = "live-momentum".into();
        variant.profile.profile_id = "momentum-v1".into();
        variant.outcomes[0].observed_at_ms = now - 1_000;
        variant.outcomes[0].market_regime = Some("trend".into());
        variant.outcomes[0].horizon_seconds = Some(300);
        let active = variant.profile.clone();
        let mut state = LearningState {
            variants: BTreeMap::from([(variant_key(&variant.profile), variant)]),
            active_profiles: BTreeMap::from([("active".into(), active)]),
            ..LearningState::default()
        };

        let snapshot = walk_forward_priority_snapshot(&state, now);
        assert_eq!(
            snapshot["BTC-USDT|live-momentum"]["freshnessStatus"],
            "fresh"
        );
        assert_eq!(
            snapshot["BTC-USDT|live-momentum|profile:momentum-v1"]["freshnessStatus"],
            "fresh"
        );
        assert_eq!(
            snapshot["BTC-USDT|live-momentum|profile:momentum-v1|regime:trend|horizon:300"]["freshnessStatus"],
            "fresh"
        );
        assert_eq!(
            snapshot["BTC-USDT|live-momentum|profile:momentum-v1|regime:trend|horizon:300"]["sampleCount"],
            1
        );
        assert!(
            !snapshot.contains_key(
                "BTC-USDT|live-momentum|profile:momentum-v1|regime:range|horizon:300"
            ),
            "a different regime remains a separate, missing evidence cohort"
        );

        state.variants.values_mut().next().unwrap().outcomes[0].observed_at_ms =
            now - 25 * 60 * 60 * 1_000;
        let stale = walk_forward_priority_snapshot(&state, now);
        assert_eq!(stale["BTC-USDT|live-momentum"]["freshnessStatus"], "stale");

        state.variants.clear();
        let missing = walk_forward_priority_snapshot(&state, now);
        assert_eq!(
            missing["BTC-USDT|live-momentum|profile:momentum-v1"]["freshnessStatus"],
            "missing"
        );
    }

    #[test]
    fn walk_forward_priority_keeps_directional_scout_history_separate() {
        let now = now_ms();
        let mut variant = scout_variant("BTC-USDT", &[0.2]);
        variant.profile.strategy = "live-momentum".into();
        variant.outcomes[0].observed_at_ms = now.saturating_sub(1_000);
        variant.outcomes[0].learning_context =
            Some("scout:v1:long:trend:BTC-USDT:live-momentum:trade".into());
        variant.outcomes[0].market_regime = Some("trend".into());
        variant.outcomes[0].horizon_seconds = Some(60);
        let profile_id = variant.profile.profile_id.clone();
        let state = LearningState {
            variants: BTreeMap::from([("candidate".into(), variant)]),
            ..LearningState::default()
        };

        let snapshot = walk_forward_priority_snapshot(&state, now);
        let directional_key = format!(
            "BTC-USDT|live-momentum|profile:{profile_id}|regime:trend|horizon:60|direction:long"
        );
        assert_eq!(snapshot[&directional_key]["sampleCount"], 1);
        assert_eq!(snapshot[&directional_key]["freshnessStatus"], "fresh");
        assert!(
            !snapshot.contains_key(&directional_key.replace("direction:long", "direction:short"))
        );
    }

    #[test]
    fn walk_forward_priority_seeds_unobserved_direction_scoped_requests_as_missing() {
        let now = now_ms();
        let active = StrategyProfile {
            profile_id: "momentum-v1".into(),
            version: 1,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::new(),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let request_id = evidence_request_key_with_direction(
            "BTC-USDT",
            "live-momentum",
            "momentum-v1",
            "trend",
            "60",
            Some("short"),
        );
        let state = LearningState {
            active_profiles: BTreeMap::from([("active".into(), active)]),
            evidence_requests: BTreeMap::from([(
                request_id,
                EvidenceRequest {
                    symbol: "BTC-USDT".into(),
                    strategy: "live-momentum".into(),
                    profile_id: "momentum-v1".into(),
                    regime: "trend".into(),
                    horizon: "60".into(),
                    direction_scope: Some("short".into()),
                    status: EvidenceRequestStatus::Pending,
                    attempts: 0,
                    created_at_ms: now,
                    updated_at_ms: now,
                    next_attempt_at_ms: now,
                    reason: None,
                    coverage_before_status: Some("missing".into()),
                    completion_source_event_id: None,
                    completion_trade_id: None,
                    completion_episode_id: None,
                },
            )]),
            ..LearningState::default()
        };

        let snapshot = walk_forward_priority_snapshot(&state, now);
        let key =
            "BTC-USDT|live-momentum|profile:momentum-v1|regime:trend|horizon:60|direction:short";
        assert_eq!(snapshot[key]["sampleCount"], 0);
        assert_eq!(snapshot[key]["effectiveSampleSize"], 0.0);
        assert_eq!(snapshot[key]["freshnessStatus"], "missing");
    }

    #[test]
    fn stale_profile_parameters_decay_toward_catalog_defaults() {
        let now = now_ms();
        let mut variant = scout_variant("BTC-USDT", &[0.01]);
        variant.outcomes[0].observed_at_ms = now.saturating_sub(PARAMETER_HALF_LIFE_MS);
        let profile = variant.profile.clone();
        let state = LearningState {
            variants: BTreeMap::from([(variant_key(&variant.profile), variant)]),
            ..LearningState::default()
        };

        let decayed = decayed_profile(&profile, &state, now);

        assert!((decayed.parameters["threshold"] - 0.6).abs() < 1e-9);
        assert_eq!(profile.parameters["threshold"], 0.7);
    }

    fn scout_variant(symbol: &str, returns: &[f64]) -> RegisteredProfile {
        RegisteredProfile {
            profile: StrategyProfile {
                profile_id: format!("{symbol}-profile"),
                version: 0,
                symbol: symbol.into(),
                strategy: "example-strategy".into(),
                parameters: BTreeMap::from([("threshold".into(), 0.7)]),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            },
            specs: vec![ParameterSpec {
                key: "threshold".into(),
                label: "Threshold".into(),
                default: 0.5,
                min: 0.0,
                max: 1.0,
                step: 0.1,
                unit: "".into(),
                learning_direction: 1.0,
            }],
            samples: returns.len(),
            positive: 0,
            realized_pnl: 0.0,
            outcomes: returns
                .iter()
                .enumerate()
                .map(|(index, value)| LearningOutcome {
                    observed_at_ms: index as u64,
                    net_return_percent: *value,
                    evidence_group_id: Some(format!("{symbol}-comparison-{index}")),
                    independence_block_id: Some(index as u64),
                    candidate_family_id: Some(format!(
                        "scout-promotion-family:v1:{}:threshold",
                        strategy::profile_id(
                            symbol,
                            "example-strategy",
                            &BTreeMap::from([("threshold".into(), 0.5)]),
                        )
                    )),
                    candidate_family_size: Some(11),
                    learning_context: None,
                    market_regime: None,
                    horizon_seconds: None,
                    source: LearningOutcomeSource::Scout,
                    realized_pnl_usd: None,
                    paired_comparison: true,
                    tested_parameter_keys: vec!["threshold".into()],
                    baseline_parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                    lineage: None,
                })
                .collect(),
        }
    }

    #[test]
    fn scout_evidence_funnel_separates_retained_rows_groups_and_unavailable_stages() {
        let now = now_ms();
        let cutoff = now.saturating_sub(7 * 24 * 60 * 60 * 1_000);
        let mut first = scout_variant("BTC-USDT", &[0.1, 0.2]);
        first.profile.strategy = "live-momentum".into();
        first.outcomes[0].observed_at_ms = now;
        first.outcomes[0].evidence_group_id = Some("paired-opportunity".into());
        first.outcomes[0].independence_block_id = Some(10);
        first.outcomes[0].candidate_family_id = Some("family-a".into());
        first.outcomes[1].observed_at_ms = now;
        first.outcomes[1].evidence_group_id = Some("single-opportunity".into());
        first.outcomes[1].paired_comparison = false;
        first.outcomes[1].independence_block_id = None;
        first.outcomes[1].candidate_family_id = None;

        let mut second = first.clone();
        second.profile.profile_id = "BTC-USDT-second-variant".into();
        second.profile.version = 1;
        second.outcomes.truncate(1);

        let mut ungrouped = first.outcomes[0].clone();
        ungrouped.observed_at_ms = now;
        ungrouped.evidence_group_id = None;
        ungrouped.independence_block_id = None;
        ungrouped.candidate_family_id = None;
        first.outcomes.push(ungrouped);

        let mut stale = first.outcomes[0].clone();
        stale.observed_at_ms = cutoff.saturating_sub(1);
        stale.evidence_group_id = Some("outside-window".into());
        first.outcomes.push(stale);

        let mut trade = first.outcomes[0].clone();
        trade.observed_at_ms = now;
        trade.source = LearningOutcomeSource::Trade;
        first.outcomes.push(trade);

        let mut state = LearningState::default();
        state.variants.insert("variant-a".into(), first);
        state.variants.insert("variant-b".into(), second);
        state.exit_timing_observations.insert(
            "exit-1".into(),
            serde_json::json!({
                "strategy": "live-momentum",
                "observed_at_ms": now
            }),
        );
        state.exit_timing_observations.insert(
            "exit-old".into(),
            serde_json::json!({
                "strategy": "live-momentum",
                "observed_at_ms": cutoff.saturating_sub(1)
            }),
        );

        let funnel = scout_evidence_funnel(&state, "live-momentum", cutoff);
        assert_eq!(funnel.retained_scout_outcome_rows, 4);
        assert_eq!(funnel.retained_paired_scout_rows, 3);
        assert_eq!(funnel.rows_with_explicit_evidence_group_id, 3);
        assert_eq!(funnel.rows_without_explicit_evidence_group_id, 1);
        assert_eq!(funnel.distinct_evidence_groups_by_symbol_strategy, 2);
        assert_eq!(funnel.distinct_paired_evidence_groups_by_symbol_strategy, 1);
        assert_eq!(funnel.groups_with_multiple_retained_rows, 1);
        assert_eq!(funnel.rows_in_groups_with_multiple_retained_rows, 2);
        assert_eq!(funnel.additional_rows_after_first_per_group, 1);
        assert_eq!(funnel.paired_rows_with_candidate_family_id, 2);
        assert_eq!(funnel.paired_rows_with_four_hour_cluster_id, 2);
        assert_eq!(funnel.distinct_symbol_four_hour_cluster_buckets, 1);
        assert_eq!(funnel.distinct_candidate_families, 1);
        assert_eq!(funnel.registered_variant_profile_attributed_rows, 4);
        assert_eq!(funnel.matching_exit_timing_observation_rows, 1);
        assert_eq!(funnel.trade_learning_outcome_rows, 1);
        assert_eq!(funnel.unique_source_event_rows, None);
        assert_eq!(funnel.excluded_ingress_rows, None);
        assert_eq!(funnel.complete_executable_path_rows, None);
        assert_eq!(funnel.cost_complete_executable_path_rows, None);
        assert_eq!(funnel.exact_decision_profile_attributed_rows, None);
        assert_eq!(funnel.training_rows, None);
        assert_eq!(funnel.holdout_rows, None);
        let contract = serde_json::to_value(&funnel).unwrap();
        assert_eq!(contract["retained_scout_outcome_rows"], 4);
        assert_eq!(contract["distinct_evidence_groups_by_symbol_strategy"], 2);
        assert_eq!(
            contract["complete_executable_path_rows"],
            serde_json::Value::Null
        );
        assert_eq!(contract["excluded_ingress_rows"], serde_json::Value::Null);
    }

    #[test]
    fn negative_upper_bound_marks_a_well_sampled_candidate_to_avoid() {
        let returns = vec![-0.2; 9];
        let sample_count = returns.len();
        let mean = returns.iter().sum::<f64>() / sample_count as f64;
        let variance = returns
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (sample_count - 1) as f64;
        let margin = one_sided_90_t_critical(sample_count).unwrap()
            * (variance / sample_count as f64).sqrt();
        assert!(mean + margin < 0.0);
    }

    #[test]
    fn scout_parameter_evidence_is_aggregated_per_symbol() {
        let mut state = LearningState::default();
        state.minimum_live_scout_parameter_samples = MINIMUM_SCOUT_PARAMETER_SAMPLES;
        state
            .variants
            .insert("a".into(), scout_variant("A", &[0.2, 0.4]));
        state
            .variants
            .insert("b".into(), scout_variant("B", &[-0.3]));

        let snapshots = scout_parameter_variant_snapshots(&state);
        let a = snapshots.iter().find(|row| row.symbol == "A").unwrap();
        let b = snapshots.iter().find(|row| row.symbol == "B").unwrap();

        assert_eq!(a.sample_count, 2);
        assert!((a.average_net_improvement_percent - 0.3).abs() < 1e-10);
        assert_eq!(a.paired_baseline_increase_samples, 2);
        assert_eq!(b.sample_count, 1);
        assert_eq!(b.average_net_improvement_percent, -0.3);
        assert_eq!(b.paired_baseline_increase_samples, 1);
        assert_eq!(b.lower_90_one_sided_improvement_percent, None);
    }

    #[test]
    fn horizon_observations_share_one_parameter_evidence_sample() {
        let mut state = LearningState::default();
        let mut variant = scout_variant("A", &[0.2, 0.4, 0.6, 0.8]);
        for outcome in &mut variant.outcomes {
            outcome.evidence_group_id = Some("comparison".into());
        }
        state.variants.insert("candidate".into(), variant);

        let snapshots = scout_parameter_variant_snapshots(&state);
        let evidence = snapshots.iter().find(|row| row.symbol == "A").unwrap();

        assert_eq!(evidence.sample_count, 1);
        assert!((evidence.average_net_improvement_percent - 0.5).abs() < 1e-10);
        assert_eq!(evidence.paired_baseline_increase_samples, 1);
    }

    #[tokio::test]
    async fn strategy_summary_separates_trade_returns_from_scout_evidence() {
        let mut state = LearningState::default();
        let mut trade = scout_variant("BTC-USDT", &[-0.02]);
        trade.profile.strategy = "example-strategy".into();
        trade.outcomes[0].source = LearningOutcomeSource::Trade;
        trade.outcomes[0].realized_pnl_usd = Some(-1.0);
        trade.outcomes[0].baseline_parameters.clear();
        trade.outcomes[0].tested_parameter_keys.clear();
        trade.outcomes[0].observed_at_ms = now_ms();
        let active_profile = trade.profile.clone();
        state.variants.insert("trade".into(), trade);
        state
            .active_profiles
            .insert("BTC-USDT:example-strategy".into(), active_profile);

        let mut previous_trade = scout_variant("BTC-USDT", &[-0.4]);
        previous_trade.profile.strategy = "example-strategy".into();
        previous_trade
            .profile
            .parameters
            .insert("threshold".into(), 0.6);
        previous_trade.outcomes[0].source = LearningOutcomeSource::Trade;
        previous_trade.outcomes[0].observed_at_ms = now_ms();
        state
            .variants
            .insert("previous-trade".into(), previous_trade);

        let mut scout = scout_variant("ETH-USDT", &[0.4]);
        scout.profile.strategy = "example-strategy".into();
        scout.outcomes[0].paired_comparison = false;
        scout.outcomes[0].baseline_parameters.clear();
        scout.outcomes[0].observed_at_ms = now_ms();
        state.variants.insert("scout".into(), scout);

        let mut paired = scout_variant("SOL-USDT", &[0.1]);
        paired.profile.strategy = "example-strategy".into();
        paired.outcomes[0].observed_at_ms = now_ms();
        paired.outcomes[0].paired_comparison = true;
        state.variants.insert("paired".into(), paired);

        let mut paper_scout = scout_variant("DOGE-USDT", &[0.05]);
        paper_scout.profile.strategy = "example-strategy".into();
        paper_scout.outcomes[0].source = LearningOutcomeSource::Scout;
        paper_scout.outcomes[0].realized_pnl_usd = Some(0.25);
        paper_scout.outcomes[0].paired_comparison = false;
        paper_scout.outcomes[0].baseline_parameters.clear();
        paper_scout.outcomes[0].tested_parameter_keys.clear();
        paper_scout.outcomes[0].observed_at_ms = now_ms();
        state.variants.insert("paper-scout".into(), paper_scout);

        let service = ServiceState {
            state: Arc::new(tokio::sync::Mutex::new(state)),
            path: PathBuf::new(),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let Json(summaries) = get_summary(State(service)).await;
        let summary = &summaries[0];

        assert_eq!(summary.learning_samples, 2);
        assert_eq!(summary.positive_rate, Some(0.0));
        assert!((summary.average_net_return_percent.unwrap() + 0.21).abs() < 1e-10);
        assert_eq!(summary.active_profile_trade_samples, 1);
        assert_eq!(
            summary.active_profile_average_net_return_percent,
            Some(-0.02)
        );
        assert_eq!(summary.other_profile_trade_samples, 1);
        assert_eq!(summary.other_profile_average_net_return_percent, Some(-0.4));
        assert_eq!(summary.scout_outcome_samples, 3);
        assert!((summary.scout_average_net_return_percent.unwrap() - (0.55 / 3.0)).abs() < 1e-10);
        assert_eq!(summary.paired_scout_samples, 1);
        assert_eq!(summary.average_paired_scout_lift_percent, Some(0.1));
    }

    #[tokio::test]
    async fn decision_snapshot_keeps_recent_real_returns_scoped_to_profile_and_context() {
        let mut state = LearningState::default();
        let mut variant = scout_variant("BTC-USDT", &[-0.1, -0.2, -0.3, 99.0]);
        let context = "long:trend:BTC-USDT:example-strategy";
        for (index, outcome) in variant.outcomes.iter_mut().enumerate() {
            outcome.observed_at_ms = now_ms() + index as u64;
            outcome.learning_context = Some(context.into());
            if index < 3 {
                outcome.source = LearningOutcomeSource::Trade;
            }
        }
        let profile_id = variant.profile.profile_id.clone();
        state.variants.insert("variant".into(), variant);
        let mut active_profile = state.variants["variant"].profile.clone();
        active_profile.profile_id = "different-active-profile".into();
        state
            .active_profiles
            .insert("BTC-USDT:example-strategy".into(), active_profile);
        let service = ServiceState {
            state: Arc::new(tokio::sync::Mutex::new(state)),
            path: PathBuf::new(),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let Json(snapshot) = get_decision_snapshot(State(service)).await;
        assert!(
            snapshot
                .snapshot
                .registered_profile_ids
                .contains(&profile_id)
        );
        assert!(
            !snapshot
                .snapshot
                .registered_profile_ids
                .contains("different-active-profile")
        );
        let samples =
            &snapshot.snapshot.recent_profile_context_returns[&format!("{profile_id}|{context}")];
        assert_eq!(
            samples.iter().map(|sample| sample.1).collect::<Vec<_>>(),
            vec![-0.3, -0.2, -0.1]
        );
    }

    #[test]
    fn startup_rebuilds_profile_stats_from_recent_closed_trades_only() {
        let mut state = LearningState::default();
        let mut trade = scout_variant("BTC-USDT", &[0.03]);
        trade.profile.strategy = "example-strategy".into();
        trade.outcomes[0].source = LearningOutcomeSource::Trade;
        trade.outcomes[0].realized_pnl_usd = Some(12.0);
        trade.outcomes[0].observed_at_ms = now_ms();
        trade.outcomes[0].baseline_parameters.clear();
        trade.outcomes[0].tested_parameter_keys.clear();
        state.variants.insert("trade".into(), trade);

        let mut scout = scout_variant("BTC-USDT", &[50.0]);
        scout.profile.strategy = "example-strategy".into();
        scout.outcomes[0].observed_at_ms = now_ms();
        state.variants.insert("scout".into(), scout);

        state.active_profiles.insert(
            "BTC-USDT:example-strategy".into(),
            StrategyProfile {
                profile_id: "active".into(),
                version: 4,
                symbol: "BTC-USDT".into(),
                strategy: "example-strategy".into(),
                parameters: BTreeMap::new(),
                learning_samples: 100,
                positive_samples: 80,
                realized_pnl: 500.0,
            },
        );

        rebuild_trade_profile_metrics(&mut state, now_ms());
        let active = &state.active_profiles["BTC-USDT:example-strategy"];
        assert_eq!(active.learning_samples, 1);
        assert_eq!(active.positive_samples, 1);
        assert_eq!(active.realized_pnl, 12.0);
        assert_eq!(active.version, 5);
    }

    #[test]
    fn evidence_gain_completion_uses_exact_request_cohort_and_attempt() {
        let now = 9_000_000;
        let request_id = evidence_request_key_with_direction(
            "BTC-USDT",
            "live-momentum",
            "profile-1",
            "trend",
            "300",
            Some("long"),
        );
        let cohort_id =
            "BTC-USDT|live-momentum|profile:profile-1|regime:trend|horizon:300|direction:long";
        let request = EvidenceRequest {
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile-1".into(),
            regime: "trend".into(),
            horizon: "300".into(),
            direction_scope: Some("long".into()),
            status: EvidenceRequestStatus::Completed,
            attempts: 2,
            created_at_ms: now - 10_000,
            updated_at_ms: now,
            next_attempt_at_ms: now,
            reason: None,
            coverage_before_status: Some("stale".into()),
            completion_source_event_id: None,
            completion_trade_id: None,
            completion_episode_id: None,
        };
        let mut state = LearningState::default();
        state.evidence_requests.insert(request_id.clone(), request);
        state.evidence_gain_records.insert(
            "gain-transition-1".into(),
            EvidenceGainRecord {
                schema_version: 2,
                record_id: "gain-transition-1".into(),
                outcome_event_id: "scout-event-3".into(),
                source_event_id: Some("scout-event-3".into()),
                trade_id: Some("trade-3".into()),
                episode_id: Some("episode-1".into()),
                profile_id: Some("profile-1".into()),
                profile_hash: Some("hash-1".into()),
                profile_version: Some(4),
                execution_mode: Some("paper".into()),
                evidence_cohort_id: Some(cohort_id.into()),
                evidence_request_id: Some(request_id.clone()),
                evidence_request_attempt: Some(2),
                observed_at_ms: now - 1,
                coverage_before_status: "stale".into(),
                coverage_after_status: "fresh".into(),
                sample_count_before: Some(1),
                sample_count_after: Some(2),
                effective_sample_size_before: Some(0.8),
                effective_sample_size_after: Some(1.4),
                last_sample_before_ms: Some(now - 100_000),
                last_sample_after_ms: Some(now - 1),
                new_coverage_observed: true,
                request_closed_at_ms: None,
                cost_status: "complete".into(),
                reconciliation_status: "unknown".into(),
                eligibility_status: "blocked".into(),
                blocked_reasons: vec!["reconciliation_unknown".into()],
            },
        );
        state.evidence_gain_records.insert(
            "wrong-cohort".into(),
            EvidenceGainRecord {
                evidence_cohort_id: Some("other-cohort".into()),
                ..state.evidence_gain_records["gain-transition-1"].clone()
            },
        );

        assert!(record_evidence_gain_request_completion(
            &mut state,
            &request_id,
            now
        ));
        let closed = &state.evidence_requests[&request_id];
        assert_eq!(
            closed.completion_source_event_id.as_deref(),
            Some("scout-event-3")
        );
        assert_eq!(closed.completion_trade_id.as_deref(), Some("trade-3"));
        assert_eq!(closed.completion_episode_id.as_deref(), Some("episode-1"));
        assert_eq!(
            state.evidence_gain_records["gain-transition-1"].request_closed_at_ms,
            Some(now)
        );
        assert!(!record_evidence_gain_request_completion(
            &mut state,
            &request_id,
            now + 1
        ));
    }
}

pub(super) async fn get_summary(State(service): State<ServiceState>) -> Json<Vec<StrategySummary>> {
    let state = service.state.lock().await;
    let cutoff = now_ms().saturating_sub(7 * 24 * 60 * 60 * 1000);
    #[derive(Default)]
    struct Returns {
        trades: Vec<f64>,
        active_profile_trades: Vec<f64>,
        other_profile_trades: Vec<f64>,
        scouts: BTreeMap<String, Vec<f64>>,
        paired_scouts: BTreeMap<String, Vec<f64>>,
    }
    let mut summaries: BTreeMap<String, Returns> = BTreeMap::new();
    for (profile_id, profile) in &state.variants {
        let is_active_profile = state
            .active_profiles
            .get(&format!(
                "{}:{}",
                profile.profile.symbol, profile.profile.strategy
            ))
            .is_some_and(|active| active.parameters == profile.profile.parameters);
        let summary = summaries
            .entry(profile.profile.strategy.clone())
            .or_default();
        for (index, outcome) in profile.outcomes.iter().enumerate().filter(|(_, outcome)| {
            outcome.observed_at_ms >= cutoff && outcome.net_return_percent.is_finite()
        }) {
            match outcome.source {
                LearningOutcomeSource::Trade => {
                    summary.trades.push(outcome.net_return_percent);
                    if is_active_profile {
                        summary
                            .active_profile_trades
                            .push(outcome.net_return_percent);
                    } else {
                        summary
                            .other_profile_trades
                            .push(outcome.net_return_percent);
                    }
                }
                LearningOutcomeSource::Scout => {
                    // Scout net is the overall Scout return, including paired
                    // comparisons. Keep a second bucket so the paired lift
                    // column can still be shown independently.
                    let group = outcome
                        .evidence_group_id
                        .clone()
                        .unwrap_or_else(|| format!("{profile_id}:{index}"));
                    summary
                        .scouts
                        .entry(group.clone())
                        .or_default()
                        .push(outcome.net_return_percent);
                    if outcome.paired_comparison {
                        summary
                            .paired_scouts
                            .entry(group)
                            .or_default()
                            .push(outcome.net_return_percent);
                    }
                }
            }
        }
    }
    Json(
        summaries
            .into_iter()
            .map(|(strategy, returns)| {
                let summarize = |values: &[f64]| {
                    let count = values.len();
                    let positive = values.iter().filter(|value| **value > 0.0).count();
                    (
                        count,
                        (count > 0).then_some(positive as f64 / count as f64),
                        (count > 0).then_some(values.iter().sum::<f64>() / count as f64),
                    )
                };
                let (trade_count, trade_positive_rate, trade_average) = summarize(&returns.trades);
                let (active_trade_count, active_trade_positive_rate, active_trade_average) =
                    summarize(&returns.active_profile_trades);
                let (other_trade_count, other_trade_positive_rate, other_trade_average) =
                    summarize(&returns.other_profile_trades);
                let grouped_returns = |groups: &BTreeMap<String, Vec<f64>>| {
                    groups
                        .values()
                        .map(|values| values.iter().sum::<f64>() / values.len() as f64)
                        .collect::<Vec<_>>()
                };
                let scouts = grouped_returns(&returns.scouts);
                let paired_scouts = grouped_returns(&returns.paired_scouts);
                let (scout_count, scout_positive_rate, scout_average) = summarize(&scouts);
                let (paired_count, paired_positive_rate, paired_average) =
                    summarize(&paired_scouts);
                let evidence_funnel = scout_evidence_funnel(&state, &strategy, cutoff);
                StrategySummary {
                    strategy,
                    learning_samples: trade_count,
                    positive_rate: trade_positive_rate,
                    average_net_return_percent: trade_average,
                    scout_outcome_samples: scout_count,
                    scout_positive_rate,
                    scout_average_net_return_percent: scout_average,
                    active_profile_trade_samples: active_trade_count,
                    active_profile_trade_positive_rate: active_trade_positive_rate,
                    active_profile_average_net_return_percent: active_trade_average,
                    other_profile_trade_samples: other_trade_count,
                    other_profile_trade_positive_rate: other_trade_positive_rate,
                    other_profile_average_net_return_percent: other_trade_average,
                    paired_scout_samples: paired_count,
                    paired_scout_positive_rate: paired_positive_rate,
                    average_paired_scout_lift_percent: paired_average,
                    evidence_funnel,
                }
            })
            .collect(),
    )
}

fn scout_evidence_funnel(
    state: &LearningState,
    strategy_name: &str,
    cutoff_ms: u64,
) -> ScoutEvidenceFunnel {
    let mut scout_rows = 0usize;
    let mut paired_rows = 0usize;
    let mut rows_with_group_id = 0usize;
    let mut rows_with_candidate_family = 0usize;
    let mut rows_with_cluster = 0usize;
    let mut trade_rows = 0usize;
    let mut profile_attributed_rows = 0usize;
    let mut group_rows = BTreeMap::<(String, String), usize>::new();
    let mut paired_groups = BTreeSet::<(String, String)>::new();
    let mut symbol_clusters = BTreeSet::<(String, u64)>::new();
    let mut candidate_families = BTreeSet::<(String, String)>::new();

    for variant in state
        .variants
        .values()
        .filter(|variant| variant.profile.strategy == strategy_name)
    {
        for outcome in variant.outcomes.iter().filter(|outcome| {
            outcome.observed_at_ms >= cutoff_ms && outcome.net_return_percent.is_finite()
        }) {
            match outcome.source {
                LearningOutcomeSource::Trade => trade_rows += 1,
                LearningOutcomeSource::Scout => {
                    scout_rows += 1;
                    // Each retained row is stored under a registered Learning
                    // variant. This is variant attribution, not proof that the
                    // source decision used that exact profile version.
                    profile_attributed_rows += 1;
                    if outcome.paired_comparison {
                        paired_rows += 1;
                        if let Some(family_id) = outcome.candidate_family_id.as_ref() {
                            rows_with_candidate_family += 1;
                            candidate_families
                                .insert((variant.profile.symbol.clone(), family_id.clone()));
                        }
                        if let Some(block_id) = outcome.independence_block_id {
                            rows_with_cluster += 1;
                            symbol_clusters.insert((variant.profile.symbol.clone(), block_id));
                        }
                    }
                    let Some(group_id) = outcome
                        .evidence_group_id
                        .as_deref()
                        .filter(|group_id| !group_id.trim().is_empty())
                    else {
                        continue;
                    };
                    rows_with_group_id += 1;
                    let group_key = (variant.profile.symbol.clone(), group_id.to_owned());
                    *group_rows.entry(group_key.clone()).or_default() += 1;
                    if outcome.paired_comparison {
                        paired_groups.insert(group_key);
                    }
                }
            }
        }
    }

    let groups_with_multiple_rows = group_rows.values().filter(|count| **count > 1).count();
    let additional_rows_after_first = group_rows
        .values()
        .map(|count| count.saturating_sub(1))
        .sum();
    let matching_exit_observation_rows = state
        .exit_timing_observations
        .values()
        .filter(|observation| {
            observation["strategy"].as_str() == Some(strategy_name)
                && observation["observed_at_ms"]
                    .as_u64()
                    .is_some_and(|observed_at_ms| observed_at_ms >= cutoff_ms)
        })
        .count();

    ScoutEvidenceFunnel {
        retained_rows_definition: "finite Scout outcome rows stored under Learning variants with observed_at_ms in the rolling seven-day window; rows are accepted retained records, not raw ingress attempts",
        group_definition: "explicit evidence_group_id deduplicated across candidate variants within this strategy and symbol; groups are comparison cohorts, not independent observations",
        four_hour_cluster_definition: "distinct symbol and UTC four-hour buckets carried by paired Scout outcomes; buckets cluster rows and are not effective independent sample counts",
        window_start_ms: cutoff_ms,
        retained_scout_outcome_rows: scout_rows,
        retained_paired_scout_rows: paired_rows,
        rows_with_explicit_evidence_group_id: rows_with_group_id,
        rows_without_explicit_evidence_group_id: scout_rows.saturating_sub(rows_with_group_id),
        distinct_evidence_groups_by_symbol_strategy: group_rows.len(),
        distinct_paired_evidence_groups_by_symbol_strategy: paired_groups.len(),
        groups_with_multiple_retained_rows: groups_with_multiple_rows,
        rows_in_groups_with_multiple_retained_rows: group_rows
            .iter()
            .filter(|(_, count)| **count > 1)
            .map(|(_, count)| *count)
            .sum(),
        additional_rows_after_first_per_group: additional_rows_after_first,
        paired_rows_with_candidate_family_id: rows_with_candidate_family,
        paired_rows_with_four_hour_cluster_id: rows_with_cluster,
        distinct_symbol_four_hour_cluster_buckets: symbol_clusters.len(),
        distinct_candidate_families: candidate_families.len(),
        registered_variant_profile_attributed_rows: profile_attributed_rows,
        matching_exit_timing_observation_rows: matching_exit_observation_rows,
        trade_learning_outcome_rows: trade_rows,
        unique_source_event_rows: None,
        excluded_ingress_rows: None,
        complete_executable_path_rows: None,
        cost_complete_executable_path_rows: None,
        exact_decision_profile_attributed_rows: None,
        training_rows: None,
        holdout_rows: None,
    }
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub(super) fn event_observed_at(event: &FeedbackEvent) -> u64 {
    match event {
        FeedbackEvent::ScoutOutcome { observed_at_ms, .. }
        | FeedbackEvent::TradeOutcome { observed_at_ms, .. }
        | FeedbackEvent::MissedOpportunityCandidate { observed_at_ms, .. } => *observed_at_ms,
    }
}

pub(super) async fn register_profile(
    State(service): State<ServiceState>,
    Json(request): Json<StrategyProfileRegistration>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !strategy::is_autotrader_symbol_allowed(&request.profile.symbol) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "symbol is outside the AutoTrader allowlist".into(),
        ));
    }
    if serde_json::to_vec(&request).map_or(true, |encoded| encoded.len() > 16 * 1024) {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "profile registration exceeds the 16 KiB limit".into(),
        ));
    }
    if request.profile.profile_id.is_empty()
        || request.profile.profile_id.len() > 128
        || request.profile.symbol.is_empty()
        || request.profile.symbol.len() > 64
        || request.profile.strategy.is_empty()
        || request.profile.strategy.len() > 64
        || request.profile.parameters.len() > 64
        || request.profile.parameters.keys().any(|key| key.len() > 128)
        || request.specs.len() > 64
        || request
            .profile
            .parameters
            .values()
            .any(|value| !value.is_finite())
        || request.specs.iter().any(|spec| !valid_spec(spec))
        || request
            .specs
            .iter()
            .any(|spec| !request.profile.parameters.contains_key(&spec.key))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid strategy profile registration".into(),
        ));
    }
    let profile_key = key(&request.profile.symbol, &request.profile.strategy);
    let mut state = service.state.lock().await;
    retention::ensure_active_profile_room(&state, &profile_key)?;
    let id = variant_key(&request.profile);
    retention::ensure_variant_room(&mut state, &id)?;
    state.variants.entry(id).or_insert(RegisteredProfile {
        profile: request.profile.clone(),
        specs: request.specs,
        samples: request.profile.learning_samples,
        positive: request.profile.positive_samples,
        realized_pnl: request.profile.realized_pnl,
        outcomes: Vec::new(),
    });
    let current = state
        .active_profiles
        .entry(profile_key)
        .or_insert(request.profile.clone());
    if request.profile.version > current.version {
        *current = request.profile;
    }
    persist(&service.path, &state)?;
    Ok(StatusCode::ACCEPTED)
}
