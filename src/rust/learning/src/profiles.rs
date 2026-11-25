use super::*;
use strategy::StrategyWalkForwardEvidence;

const OUTCOME_HALF_LIFE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const PARAMETER_HALF_LIFE_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

fn outcome_decay_weight(observed_at_ms: u64, now_ms: u64) -> f64 {
    let age_ms = now_ms.saturating_sub(observed_at_ms);
    0.5_f64.powf(age_ms as f64 / OUTCOME_HALF_LIFE_MS as f64)
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
            let keys = [
                cohort.clone(),
                exact_profile.clone(),
                regime_cohort,
                regime_profile,
                horizon_cohort,
                horizon_profile,
                exact_case_cohort,
                exact_case_profile,
            ];
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
    let exit_protection_policies = profiles
        .iter()
        .filter_map(|profile| {
            let evidence = crate::exit_policy::select_exit_policy(
                state.exit_timing_observations.values(),
                &profile.symbol,
                &profile.strategy,
                snapshot_now_ms,
            )?;
            Some(strategy::LearnedExitProtectionPolicy {
                symbol: profile.symbol.clone(),
                strategy: profile.strategy.clone(),
                stop_loss_percent: Some(evidence.stop_loss_percent),
                take_profit_percent: Some(evidence.take_profit_percent),
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
            })
        })
        .collect();
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
    let mut outcomes = BTreeMap::<CandidateKey, BTreeMap<String, Vec<f64>>>::new();
    let mut baseline_directions = BTreeMap::<CandidateKey, [usize; 4]>::new();
    let mut baseline_deltas = BTreeMap::<CandidateKey, (f64, usize)>::new();
    let mut direction_groups = BTreeSet::<(CandidateKey, String)>::new();
    let mut delta_groups = BTreeSet::<(CandidateKey, String)>::new();
    for (profile_id, variant) in &state.variants {
        for (outcome_index, outcome) in variant.outcomes.iter().enumerate() {
            // Multi-parameter comparisons are promoted as a tested group, so
            // don't present their returns as isolated single-parameter tests.
            if outcome.tested_parameter_keys.len() != 1 {
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
            outcomes
                .entry(key.clone())
                .or_default()
                .entry(
                    outcome
                        .evidence_group_id
                        .clone()
                        .unwrap_or_else(|| format!("{profile_id}:{outcome_index}")),
                )
                .or_default()
                .push(outcome.net_return_percent);
            let group_id = outcome
                .evidence_group_id
                .clone()
                .unwrap_or_else(|| format!("{profile_id}:{outcome_index}"));
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
    for (key, grouped_observations) in outcomes {
        let (symbol, strategy, parameter, value_string, market_regime, baseline_profile_id) =
            key.clone();
        let Ok(candidate_value) = value_string.parse::<f64>() else {
            continue;
        };
        if grouped_observations.is_empty() {
            continue;
        }
        let returns = grouped_observations
            .values()
            .map(|observations| observations.iter().sum::<f64>() / observations.len() as f64)
            .collect::<Vec<_>>();
        let sample_count = returns.len();
        let direction_counts = baseline_directions.get(&key).copied().unwrap_or_default();
        let (paired_delta_sum, paired_delta_samples) =
            baseline_deltas.get(&key).copied().unwrap_or_default();
        let paired_delta_mean =
            (paired_delta_samples > 0).then_some(paired_delta_sum / paired_delta_samples as f64);
        let minimum_samples = state.minimum_live_scout_parameter_samples;
        let mean = returns.iter().sum::<f64>() / sample_count as f64;
        let (lower_bound, upper_bound) = if sample_count >= minimum_samples && sample_count > 1 {
            let variance = returns
                .iter()
                .map(|value| (value - mean).powi(2))
                .sum::<f64>()
                / (sample_count - 1) as f64;
            let interval = one_sided_90_t_critical(sample_count).map(|critical| {
                let margin = critical * (variance / sample_count as f64).sqrt();
                (mean - margin, mean + margin)
            });
            interval.map_or((None, None), |(lower, upper)| (Some(lower), Some(upper)))
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
                    evidence_group_id: None,
                    learning_context: None,
                    market_regime: None,
                    horizon_seconds: None,
                    source: LearningOutcomeSource::Scout,
                    realized_pnl_usd: None,
                    paired_comparison: false,
                    tested_parameter_keys: vec!["threshold".into()],
                    baseline_parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                    lineage: None,
                })
                .collect(),
        }
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
                }
            })
            .collect(),
    )
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
