//! Point-in-time replay for Learning-owned profit-protection policy selection.
//!
//! This module deliberately fails closed: observations missing the effective
//! baseline parameters, executable quote path, fee data, or observation time
//! are excluded. The persisted live path is sampled (typically once per
//! minute after exit), so trigger timing is approximate at quote cadence. A
//! caller must wire it into profile generation and only publish a selected
//! profile when `select_exit_policy` returns `Some`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MIN_OBSERVATIONS: usize = 40;
const MIN_TRAIN_OBSERVATIONS: usize = 30;
const MIN_VALIDATION_OBSERVATIONS: usize = 10;
const MIN_VALIDATION_GAIN_PERCENT: f64 = 0.02;
const MAX_VALIDATION_LOSS_PERCENT: f64 = 0.01;
const MAX_VALIDATION_NEGATIVE_DIFFERENCE_RATE: f64 = 0.60;
const MIN_VALIDATION_TENTH_PERCENTILE_DIFFERENCE_PERCENT: f64 = -0.25;
const MIN_VALIDATION_WORST_DIFFERENCE_PERCENT: f64 = -0.50;
const MAX_CANDIDATES_PER_PARAMETER: usize = 16;
const MIN_POST_EXIT_PATH_SAMPLES: usize = 10;
const MIN_HORIZON_COVERAGE_PERCENT: u64 = 80;
const MAX_POST_EXIT_PATH_GAP_PERCENT: u64 = 20;
const MAX_COUNTERFACTUAL_EXIT_FEE_RATE: f64 = 0.1;
const MS_PER_DAY: f64 = 86_400_000.0;

const ACTIVATION_KEY: &str = "profit_protection_activation_percent";
const GIVEBACK_KEY: &str = "profit_protection_giveback_percent";
const MINIMUM_LOCK_KEY: &str = "profit_protection_minimum_lock_percent";
const TRAILING_ACTIVATION_KEY: &str = "trailing_activation_percent";
const TRAILING_DISTANCE_KEY: &str = "trailing_distance_percent";
const BREAKEVEN_ACTIVATION_KEY: &str = "breakeven_activation_percent";
const MAX_EXIT_PARAMETER_PERCENT: f64 = 10.0;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ProfitProtectionParameters {
    pub stop_loss_percent: f64,
    pub take_profit_percent: f64,
    pub activation_percent: f64,
    pub giveback_percent: f64,
    pub minimum_lock_percent: f64,
    pub trailing_activation_percent: Option<f64>,
    pub trailing_distance_percent: Option<f64>,
    pub breakeven_activation_percent: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ExitPolicyEvidence {
    pub profile_id: Option<String>,
    pub profile_version: Option<u64>,
    pub as_of_ms: u64,
    pub sample_count: usize,
    pub training_count: usize,
    pub validation_count: usize,
    pub baseline_validation_mean_percent: f64,
    pub selected_validation_mean_percent: f64,
    pub validation_gain_percent: f64,
    pub validation_positive_rate: f64,
    pub validation_negative_difference_rate: f64,
    pub validation_tenth_percentile_difference_percent: f64,
    pub validation_worst_difference_percent: f64,
    pub stop_loss_percent: f64,
    pub take_profit_percent: f64,
    pub parameters: ProfitProtectionParameters,
    pub maximum_holding_duration_ms: Option<u64>,
    pub metrics: strategy::ExitPolicyMetricReport,
}

#[derive(Clone, Debug)]
struct ReplayObservation {
    event_id: String,
    exit_timestamp_ms: u64,
    observed_at_ms: u64,
    direction: f64,
    entry_price: f64,
    quantity: f64,
    entry_fees: f64,
    initial_risk_quote: Option<f64>,
    initial_risk_currency: Option<String>,
    quote_currency: Option<String>,
    market_regime: Option<String>,
    entry_timestamp_ms: Option<u64>,
    maximum_holding_duration_days: Option<f64>,
    profile_id: Option<String>,
    profile_version: Option<u64>,
    initial_entry_notional_quote: Option<f64>,
    entry_fill_timing: Option<Vec<EntryFillTiming>>,
    execution_costs_complete: bool,
    counterfactual_exit_fee_attribution: CounterfactualExitFeeAttribution,
    gross_executable_mfe_percent: f64,
    gross_executable_mae_percent: f64,
    net_executable_mfe_percent: f64,
    net_executable_mae_percent: f64,
    measured_slippage_quote: Option<f64>,
    measured_slippage_currency: Option<String>,
    independent_reconciliation_complete: bool,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    baseline: ProfitProtectionParameters,
    #[allow(dead_code)]
    actual_exit_net_return_percent: f64,
    #[allow(dead_code)]
    exit_reason: String,
    path: Vec<Quote>,
}

#[derive(Clone, Debug)]
struct EntryFillTiming {
    timestamp_ms: u64,
    notional_quote: f64,
}

#[derive(Clone, Copy, Debug)]
struct Quote {
    timestamp_ms: u64,
    bid: f64,
    ask: f64,
}

#[derive(Clone, Debug)]
struct CounterfactualExitFeeAttribution {
    rate_by_timestamp: BTreeMap<u64, f64>,
    fee_currency_by_timestamp: BTreeMap<u64, String>,
    quote_currency: String,
    fx_rate_to_quote_by_timestamp: BTreeMap<u64, Option<f64>>,
}

#[derive(Clone, Copy, Debug)]
struct CandidateScore {
    parameters: ProfitProtectionParameters,
    maximum_holding_duration_days: Option<f64>,
    training_mean: f64,
}

#[derive(Clone, Copy)]
enum ExitPolicyDimension {
    StopLoss,
    TakeProfit,
    Activation,
    Giveback,
    MinimumLock,
    TrailingActivation,
    TrailingDistance,
    BreakevenActivation,
}

impl ExitPolicyDimension {
    const ALL: [Self; 8] = [
        Self::StopLoss,
        Self::TakeProfit,
        Self::Activation,
        Self::Giveback,
        Self::MinimumLock,
        Self::TrailingActivation,
        Self::TrailingDistance,
        Self::BreakevenActivation,
    ];
}

impl ProfitProtectionParameters {
    fn set(&mut self, dimension: ExitPolicyDimension, value: f64) {
        match dimension {
            ExitPolicyDimension::StopLoss => self.stop_loss_percent = value,
            ExitPolicyDimension::TakeProfit => self.take_profit_percent = value,
            ExitPolicyDimension::Activation => self.activation_percent = value,
            ExitPolicyDimension::Giveback => self.giveback_percent = value,
            ExitPolicyDimension::MinimumLock => self.minimum_lock_percent = value,
            ExitPolicyDimension::TrailingActivation => {
                self.trailing_activation_percent = Some(value);
            }
            ExitPolicyDimension::TrailingDistance => {
                self.trailing_distance_percent = Some(value);
            }
            ExitPolicyDimension::BreakevenActivation => {
                self.breakeven_activation_percent = Some(value);
            }
        }
    }
}

struct EmpiricalPolicyCandidates([Vec<f64>; 8]);

impl EmpiricalPolicyCandidates {
    fn values(&self, dimension: ExitPolicyDimension) -> &[f64] {
        match dimension {
            ExitPolicyDimension::StopLoss => &self.0[0],
            ExitPolicyDimension::TakeProfit => &self.0[1],
            ExitPolicyDimension::Activation => &self.0[2],
            ExitPolicyDimension::Giveback => &self.0[3],
            ExitPolicyDimension::MinimumLock => &self.0[4],
            ExitPolicyDimension::TrailingActivation => &self.0[5],
            ExitPolicyDimension::TrailingDistance => &self.0[6],
            ExitPolicyDimension::BreakevenActivation => &self.0[7],
        }
    }
}

fn median_baseline(samples: &[ReplayObservation]) -> Option<ProfitProtectionParameters> {
    let median = |mut values: Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values.get(values.len() / 2).copied()
    };
    let median_optional = |values: Vec<Option<f64>>| -> Option<Option<f64>> {
        if values.iter().any(Option::is_none) {
            return Some(None);
        }
        median(values.into_iter().flatten().collect()).map(Some)
    };
    Some(ProfitProtectionParameters {
        stop_loss_percent: median(
            samples
                .iter()
                .map(|sample| sample.stop_loss_percent)
                .collect(),
        )?,
        take_profit_percent: median(
            samples
                .iter()
                .map(|sample| sample.take_profit_percent)
                .collect(),
        )?,
        activation_percent: median(
            samples
                .iter()
                .map(|sample| sample.baseline.activation_percent)
                .collect(),
        )?,
        giveback_percent: median(
            samples
                .iter()
                .map(|sample| sample.baseline.giveback_percent)
                .collect(),
        )?,
        minimum_lock_percent: median(
            samples
                .iter()
                .map(|sample| sample.baseline.minimum_lock_percent)
                .collect(),
        )?,
        trailing_activation_percent: median_optional(
            samples
                .iter()
                .map(|s| s.baseline.trailing_activation_percent)
                .collect(),
        )?,
        trailing_distance_percent: median_optional(
            samples
                .iter()
                .map(|s| s.baseline.trailing_distance_percent)
                .collect(),
        )?,
        breakeven_activation_percent: median_optional(
            samples
                .iter()
                .map(|s| s.baseline.breakeven_activation_percent)
                .collect(),
        )?,
    })
}

fn empirical_policy_candidates(samples: &[ReplayObservation]) -> EmpiricalPolicyCandidates {
    let mut values: [Vec<f64>; 8] = std::array::from_fn(|_| Vec::new());
    for sample in samples {
        values[0].push(sample.stop_loss_percent);
        values[1].push(sample.take_profit_percent);
        values[2].push(sample.baseline.activation_percent);
        values[3].push(sample.baseline.giveback_percent);
        values[4].push(sample.baseline.minimum_lock_percent);
        values[5].extend(sample.baseline.trailing_activation_percent);
        values[6].extend(sample.baseline.trailing_distance_percent);
        values[7].extend(sample.baseline.breakeven_activation_percent);

        let mut peak = f64::NEG_INFINITY;
        for quote in &sample.path {
            let Some(net) = replay_quote_return(sample, quote) else {
                continue;
            };
            peak = peak.max(net);
            if net < 0.0 {
                values[0].push(-net);
            } else {
                values[1].push(net);
                values[2].push(net);
                values[4].push(net);
                values[5].push(net);
                values[7].push(net);
            }
            let giveback = peak - net;
            if giveback > 0.0 {
                values[3].push(giveback);
                values[6].push(giveback);
            }
        }
    }
    for (index, candidates) in values.iter_mut().enumerate() {
        candidates.retain(|value| {
            value.is_finite()
                && if index == 4 {
                    *value >= 0.0
                } else {
                    *value > 0.0 && (index < 5 || *value <= MAX_EXIT_PARAMETER_PERCENT)
                }
        });
        candidates.sort_by(f64::total_cmp);
        candidates.dedup_by(|left, right| (*left - *right).abs() <= f64::EPSILON);
        if candidates.len() > MAX_CANDIDATES_PER_PARAMETER {
            let original = std::mem::take(candidates);
            let last = original.len() - 1;
            candidates.extend((0..MAX_CANDIDATES_PER_PARAMETER).map(|index| {
                let position = index * last / (MAX_CANDIDATES_PER_PARAMETER - 1);
                original[position]
            }));
            candidates.dedup_by(|left, right| (*left - *right).abs() <= f64::EPSILON);
        }
    }
    EmpiricalPolicyCandidates(values)
}

fn maximum_hold_duration_candidates(
    samples: &[ReplayObservation],
    strategy_name: &str,
) -> Vec<f64> {
    let Some((_, minimum, maximum, step)) = maximum_hold_days_spec(strategy_name) else {
        return Vec::new();
    };
    if samples.is_empty()
        || samples.iter().any(|sample| {
            sample.maximum_holding_duration_days.is_none() || sample.entry_timestamp_ms.is_none()
        })
    {
        return Vec::new();
    }
    let count =
        (((maximum - minimum) / step).floor() as usize + 1).min(MAX_CANDIDATES_PER_PARAMETER);
    let full_count = ((maximum - minimum) / step).floor() as usize + 1;
    if count == 0 || full_count == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|index| {
            let source_index = if count == 1 {
                0
            } else {
                index * (full_count - 1) / (count - 1)
            };
            minimum + source_index as f64 * step
        })
        .filter(|days| valid_maximum_hold_days(strategy_name, *days))
        .collect()
}

fn baseline_maximum_hold_duration(samples: &[ReplayObservation]) -> Option<f64> {
    if samples.is_empty()
        || samples
            .iter()
            .any(|sample| sample.maximum_holding_duration_days.is_none())
    {
        return None;
    }
    let mut values = samples
        .iter()
        .filter_map(|sample| sample.maximum_holding_duration_days)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

fn valid_policy_parameters(parameters: ProfitProtectionParameters) -> bool {
    [
        parameters.stop_loss_percent,
        parameters.take_profit_percent,
        parameters.activation_percent,
        parameters.giveback_percent,
        parameters.minimum_lock_percent,
    ]
    .into_iter()
    .all(|value| value.is_finite())
        && parameters.stop_loss_percent > 0.0
        && parameters.take_profit_percent > 0.0
        && parameters.activation_percent > 0.0
        && parameters.giveback_percent > 0.0
        && parameters.minimum_lock_percent >= 0.0
        && parameters.minimum_lock_percent <= parameters.activation_percent
        && parameters.trailing_activation_percent.is_some()
            == parameters.trailing_distance_percent.is_some()
        && [
            parameters.trailing_activation_percent,
            parameters.trailing_distance_percent,
            parameters.breakeven_activation_percent,
        ]
        .into_iter()
        .flatten()
        .all(|value| value.is_finite() && value > 0.0 && value <= MAX_EXIT_PARAMETER_PERCENT)
}

/// Assess empirical policy candidates against complete executable paths.
/// Candidate hold duration is evaluated only when every training sample has
/// a valid entry timestamp and captured profile duration in the catalog range.
pub(crate) fn select_exit_policy<'a, I>(
    observations: I,
    symbol: &str,
    strategy_name: &str,
    cutoff_timestamp_ms: u64,
) -> Option<ExitPolicyEvidence>
where
    I: IntoIterator<Item = &'a Value>,
{
    strategy::Strategy::from_name(strategy_name)?;
    let mut samples = observations
        .into_iter()
        .filter_map(|value| parse_observation(value, symbol, strategy_name, cutoff_timestamp_ms))
        .collect::<Vec<_>>();
    samples.sort_by_key(|sample| (sample.exit_timestamp_ms, sample.observed_at_ms));
    let mut seen_event_ids = BTreeSet::new();
    samples.retain(|sample| seen_event_ids.insert(sample.event_id.clone()));
    if samples.len() < MIN_OBSERVATIONS {
        return None;
    }

    // Reserve the newest chronological quarter as an untouched validation set.
    let validation_count = (samples.len() / 4).max(MIN_VALIDATION_OBSERVATIONS);
    let split = samples.len().checked_sub(validation_count)?;
    if split < MIN_TRAIN_OBSERVATIONS || samples.len() - split < MIN_VALIDATION_OBSERVATIONS {
        return None;
    }
    let (training, validation) = samples.split_at(split);
    let historical_baseline_score = baseline_score(training)?;
    let baseline_parameters = median_baseline(training)?;
    let baseline_hold_duration_days = baseline_maximum_hold_duration(training);
    let mut best = CandidateScore {
        parameters: baseline_parameters,
        maximum_holding_duration_days: baseline_hold_duration_days,
        training_mean: score_with_hold_duration(
            training,
            baseline_parameters,
            baseline_hold_duration_days,
        )?,
    };
    let candidates = empirical_policy_candidates(training);
    let hold_duration_candidates = maximum_hold_duration_candidates(training, strategy_name);
    let mut improved = true;
    while improved {
        improved = false;
        for dimension in ExitPolicyDimension::ALL {
            if matches!(
                dimension,
                ExitPolicyDimension::TrailingActivation | ExitPolicyDimension::TrailingDistance
            ) {
                continue;
            }
            for value in candidates.values(dimension) {
                let mut parameters = best.parameters;
                parameters.set(dimension, *value);
                if !valid_policy_parameters(parameters) {
                    continue;
                }
                let Some(candidate_mean) = score_with_hold_duration(
                    training,
                    parameters,
                    best.maximum_holding_duration_days,
                ) else {
                    continue;
                };
                if candidate_mean > best.training_mean + f64::EPSILON {
                    best = CandidateScore {
                        parameters,
                        maximum_holding_duration_days: best.maximum_holding_duration_days,
                        training_mean: candidate_mean,
                    };
                    improved = true;
                }
            }
        }
        for activation in candidates.values(ExitPolicyDimension::TrailingActivation) {
            for distance in candidates.values(ExitPolicyDimension::TrailingDistance) {
                let mut parameters = best.parameters;
                parameters.trailing_activation_percent = Some(*activation);
                parameters.trailing_distance_percent = Some(*distance);
                if !valid_policy_parameters(parameters) {
                    continue;
                }
                let Some(candidate_mean) = score_with_hold_duration(
                    training,
                    parameters,
                    best.maximum_holding_duration_days,
                ) else {
                    continue;
                };
                if candidate_mean > best.training_mean + f64::EPSILON {
                    best = CandidateScore {
                        parameters,
                        maximum_holding_duration_days: best.maximum_holding_duration_days,
                        training_mean: candidate_mean,
                    };
                    improved = true;
                }
            }
        }
        for days in &hold_duration_candidates {
            let Some(candidate_mean) =
                score_with_hold_duration(training, best.parameters, Some(*days))
            else {
                continue;
            };
            if candidate_mean > best.training_mean + f64::EPSILON {
                best = CandidateScore {
                    parameters: best.parameters,
                    maximum_holding_duration_days: Some(*days),
                    training_mean: candidate_mean,
                };
                improved = true;
            }
        }
    }

    // Require an in-sample gain over the per-position captured baseline before
    // considering the untouched chronological validation block.
    let training_candidate_improved = best.training_mean > historical_baseline_score;

    // Require paired validation improvement and a one-sided 95% lower bound
    // above the baseline. A positive mean alone is not enough to promote.
    let paired = validation
        .iter()
        .filter_map(|sample| {
            Some((
                sample,
                replay_net_return(sample, sample.baseline)?,
                replay_net_return_with_hold_duration(
                    sample,
                    best.parameters,
                    best.maximum_holding_duration_days,
                )?,
                replay_outcome_with_hold_duration(
                    sample,
                    best.parameters,
                    best.maximum_holding_duration_days,
                )?,
            ))
        })
        .collect::<Vec<_>>();
    if paired.len() < MIN_VALIDATION_OBSERVATIONS {
        return None;
    }
    let baseline_mean = paired.iter().map(|pair| pair.1).sum::<f64>() / paired.len() as f64;
    let selected_mean = paired.iter().map(|pair| pair.2).sum::<f64>() / paired.len() as f64;
    let differences = paired
        .iter()
        .map(|pair| pair.2 - pair.1)
        .collect::<Vec<_>>();
    let gain = selected_mean - baseline_mean;
    let positive_rate =
        differences.iter().filter(|value| **value > 0.0).count() as f64 / differences.len() as f64;
    let negative_difference_rate =
        differences.iter().filter(|value| **value < 0.0).count() as f64 / differences.len() as f64;
    let mut sorted_differences = differences.clone();
    sorted_differences.sort_by(f64::total_cmp);
    let tenth_percentile_difference = nearest_rank_percentile(&sorted_differences, 0.10)?;
    let worst_difference = *sorted_differences.first()?;
    let lower_bound = mean_lower_bound_95(&differences)?;
    let mut metrics =
        exit_policy_metrics(&paired, strategy_name, best.maximum_holding_duration_days);
    let profile_identity_complete = samples.first().is_some_and(|first| {
        first
            .profile_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
            && first.profile_version.is_some()
            && samples.iter().all(|sample| {
                sample.profile_id == first.profile_id
                    && sample.profile_version == first.profile_version
            })
    });
    if !profile_identity_complete {
        metrics
            .promotion_blockers
            .push("profile_version_identity_unavailable_or_mixed".to_owned());
    }
    if !training_candidate_improved {
        metrics
            .promotion_blockers
            .push("training_expectancy_did_not_improve_over_captured_baseline".to_owned());
    }
    if gain < MIN_VALIDATION_GAIN_PERCENT {
        metrics
            .promotion_blockers
            .push("validation_gain_below_minimum".to_owned());
    }
    if lower_bound <= 0.0 {
        metrics
            .promotion_blockers
            .push("validation_confidence_interval_not_positive".to_owned());
    }
    if selected_mean < baseline_mean - MAX_VALIDATION_LOSS_PERCENT {
        metrics
            .promotion_blockers
            .push("validation_mean_exceeds_allowed_loss".to_owned());
    }
    if !passes_downside_gate(
        negative_difference_rate,
        tenth_percentile_difference,
        worst_difference,
    ) {
        metrics
            .promotion_blockers
            .push("validation_downside_gate_failed".to_owned());
    }
    metrics.promotion_blockers.sort();
    metrics.promotion_blockers.dedup();
    metrics.promotion_eligible = metrics.promotion_blockers.is_empty();
    Some(ExitPolicyEvidence {
        profile_id: samples.first().and_then(|sample| sample.profile_id.clone()),
        profile_version: samples.first().and_then(|sample| sample.profile_version),
        as_of_ms: samples.iter().map(|sample| sample.observed_at_ms).max()?,
        sample_count: samples.len(),
        training_count: training.len(),
        validation_count: paired.len(),
        baseline_validation_mean_percent: baseline_mean,
        selected_validation_mean_percent: selected_mean,
        validation_gain_percent: gain,
        validation_positive_rate: positive_rate,
        validation_negative_difference_rate: negative_difference_rate,
        validation_tenth_percentile_difference_percent: tenth_percentile_difference,
        validation_worst_difference_percent: worst_difference,
        stop_loss_percent: best.parameters.stop_loss_percent,
        take_profit_percent: best.parameters.take_profit_percent,
        parameters: best.parameters,
        maximum_holding_duration_ms: if metrics.coverage["maximum_holding_duration_ms"].status
            == strategy::ExitPolicyMetricCoverageStatus::Complete
        {
            metrics.values["maximum_holding_duration_ms"]
                .is_finite()
                .then(|| metrics.values["maximum_holding_duration_ms"].ceil() as u64)
        } else {
            None
        },
        metrics,
    })
}

fn observation_matches_profile_version(
    value: &Value,
    profile_id: &str,
    profile_version: u64,
) -> bool {
    let provenance_id = value["exit_parameter_provenance"]["profile_id"].as_str();
    let decision_id = value["entry_context"]["strategy_decision"]["learning_profile_id"].as_str();
    let provenance_version = value["exit_parameter_provenance"]["profile_version"].as_u64();
    let decision_version =
        value["entry_context"]["strategy_decision"]["learning_profile_version"].as_u64();
    if provenance_id
        .zip(decision_id)
        .is_some_and(|(left, right)| left != right)
        || provenance_version
            .zip(decision_version)
            .is_some_and(|(left, right)| left != right)
    {
        return false;
    }
    provenance_id.or(decision_id) == Some(profile_id)
        && provenance_version.or(decision_version) == Some(profile_version)
}

pub(crate) fn select_exit_policy_for_profile<'a, I>(
    observations: I,
    symbol: &str,
    strategy_name: &str,
    profile_id: &str,
    profile_version: u64,
    cutoff_timestamp_ms: u64,
) -> Option<ExitPolicyEvidence>
where
    I: IntoIterator<Item = &'a Value>,
{
    let exact_profile_observations = observations
        .into_iter()
        .filter(|value| {
            value["symbol"].as_str() == Some(symbol)
                && value["strategy"].as_str() == Some(strategy_name)
                && observation_matches_profile_version(value, profile_id, profile_version)
        })
        .collect::<Vec<_>>();
    select_exit_policy(
        exact_profile_observations.iter().copied(),
        symbol,
        strategy_name,
        cutoff_timestamp_ms,
    )
}

pub(crate) fn unavailable_exit_policy_assessment_for_profile<'a, I>(
    observations: I,
    symbol: &str,
    strategy_name: &str,
    profile_id: &str,
    profile_version: u64,
    cutoff_timestamp_ms: u64,
) -> strategy::LearnedExitPolicyAssessment
where
    I: IntoIterator<Item = &'a Value>,
{
    let matching = observations
        .into_iter()
        .filter(|value| {
            value["symbol"].as_str() == Some(symbol)
                && value["strategy"].as_str() == Some(strategy_name)
        })
        .collect::<Vec<_>>();
    let exact_profile_observations = matching
        .iter()
        .copied()
        .filter(|value| observation_matches_profile_version(value, profile_id, profile_version))
        .collect::<Vec<_>>();
    let mut assessment = unavailable_exit_policy_assessment(
        exact_profile_observations.iter().copied(),
        symbol,
        strategy_name,
        cutoff_timestamp_ms,
    );
    assessment.profile_id = Some(profile_id.to_owned());
    assessment.profile_version = Some(profile_version);
    if exact_profile_observations.len() < matching.len() && !matching.is_empty() {
        assessment
            .metrics
            .promotion_blockers
            .push("broader_profile_evidence_not_pooled".to_owned());
        assessment.metrics.coverage.insert(
            "exact_profile_replay_coverage".to_owned(),
            metric_coverage(
                strategy::ExitPolicyMetricCoverageStatus::Partial,
                exact_profile_observations.len(),
                &["exit_parameter_provenance.profile_id", "exit_parameter_provenance.profile_version"],
                Some("other profile versions exist but are not pooled into this profile-specific assessment"),
            ),
        );
    }
    assessment.metrics.promotion_blockers.sort();
    assessment.metrics.promotion_blockers.dedup();
    assessment
}

/// Produce an explicit no-candidate assessment for the profile snapshot when
/// the selector cannot produce a score. This keeps insufficient path/fee
/// coverage visible without inventing fallback parameters or zero metrics.
pub(crate) fn unavailable_exit_policy_assessment<'a, I>(
    observations: I,
    symbol: &str,
    strategy_name: &str,
    cutoff_timestamp_ms: u64,
) -> strategy::LearnedExitPolicyAssessment
where
    I: IntoIterator<Item = &'a Value>,
{
    use strategy::ExitPolicyMetricCoverageStatus::Unavailable;

    let matching = observations
        .into_iter()
        .filter(|value| {
            value["symbol"].as_str() == Some(symbol)
                && value["strategy"].as_str() == Some(strategy_name)
        })
        .collect::<Vec<_>>();
    let complete_samples = matching
        .iter()
        .filter(|value| {
            parse_observation(value, symbol, strategy_name, cutoff_timestamp_ms).is_some()
        })
        .count();
    let mut metrics = strategy::ExitPolicyMetricReport::default();
    let reason = if matching.is_empty() {
        "no matching executable exit observations are available"
    } else if complete_samples < MIN_OBSERVATIONS {
        "fewer than the minimum complete fee- and path-covered observations are available"
    } else {
        "no candidate score passed the effective-sample and chronological validation prerequisites"
    };
    for key in [
        "net_expectancy_percent",
        "expectancy_r",
        "profit_factor",
        "payoff_ratio",
        "average_winner_percent",
        "average_loser_percent",
        "gross_executable_mfe_percent",
        "gross_executable_mae_percent",
        "net_executable_mfe_percent",
        "net_executable_mae_percent",
        "counterfactual_exit_fee_schedule_source_coverage_percent",
        "winner_preservation_rate",
        "loser_recovery_rate",
        "max_trade_sequence_drawdown_percent_points",
        "mean_to_sample_standard_deviation",
        "mean_position_duration_ms",
        "maximum_holding_duration_ms",
        "capital_efficiency",
        "modeled_fee_burden_percent",
        "measured_slippage_percent_of_notional",
        "account_currency_expectancy_percent",
        "worst_market_regime_improvement_percent",
        "worst_direction_improvement_percent",
        "independently_reconciled_sample_coverage_percent",
    ] {
        metrics.coverage.insert(
            key.to_owned(),
            metric_coverage(Unavailable, complete_samples, &[], Some(reason)),
        );
    }
    let gross_only_excursions = matching
        .iter()
        .filter_map(|value| gross_only_excursion_for_observation(value, symbol, strategy_name))
        .collect::<Vec<_>>();
    let fee_schedule_covered = matching
        .iter()
        .filter(|value| counterfactual_exit_fee_contract_complete(value))
        .count();
    for (key, index) in [
        ("gross_executable_mfe_percent", 0),
        ("gross_executable_mae_percent", 1),
    ] {
        let values = gross_only_excursions
            .iter()
            .map(|excursion| excursion[index])
            .collect::<Vec<_>>();
        let mean = (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64);
        add_metric(
            &mut metrics,
            key,
            mean,
            metric_coverage(
                if matching.is_empty() || values.is_empty() {
                    Unavailable
                } else if values.len() == matching.len() {
                    strategy::ExitPolicyMetricCoverageStatus::Complete
                } else {
                    strategy::ExitPolicyMetricCoverageStatus::Partial
                },
                values.len(),
                &[
                    "pre_exit_price_path.bid",
                    "pre_exit_price_path.ask",
                    "entry_context.average_price",
                ],
                Some(
                    "gross excursion only; fee-adjusted attribution requires a sourced versioned schedule",
                ),
            ),
        );
    }
    let fee_covered = matching
        .iter()
        .filter(|value| {
            let quote_currency = value["symbol"]
                .as_str()
                .and_then(|symbol| symbol.rsplit_once('-').map(|(_, quote)| quote));
            entry_fee_quote(&value["entry_context"], quote_currency).is_some()
        })
        .count();
    let path_covered = matching
        .iter()
        .filter(|value| {
            value["pre_exit_price_path_evidence"]["complete"].as_bool() == Some(true)
                && value["price_path_evidence"]["complete"].as_bool() == Some(true)
                && is_supported_quote_path(&value["pre_exit_price_path"])
                && is_supported_quote_path(&value["price_path"])
        })
        .count();
    let initial_risk_covered = matching
        .iter()
        .filter(|value| {
            value["entry_context"]["initial_risk_reference"]["schema_version"].as_u64() == Some(1)
                && value["entry_context"]["initial_risk_reference"]["status"].as_str()
                    == Some("complete")
                && value["realized_r_evidence"]["status"].as_str() == Some("complete")
        })
        .count();
    let measured_cost_covered = matching
        .iter()
        .filter(|value| {
            value["measured_slippage_quote"].as_f64().is_some()
                && value["measured_slippage_currency"].as_str().is_some()
                && value["execution_costs_complete"].as_bool() == Some(true)
        })
        .count();
    let fx_covered = matching
        .iter()
        .filter(|value| {
            value["fx_path"]
                .as_array()
                .is_some_and(|path| !path.is_empty())
        })
        .count();
    let reconciled = matching
        .iter()
        .filter(|value| {
            value["independent_reconciliation"]["status"].as_str() == Some("reconciled")
                && value["independent_reconciliation"]["source"].as_str()
                    == Some("paper_exchange_execution_ledger")
                && value["independent_reconciliation"]["evidence_epoch_id"]
                    .as_str()
                    .is_some()
        })
        .count();
    for (key, covered, source, unavailable_reason) in [
        (
            "counterfactual_exit_fee_schedule_source_coverage_percent",
            fee_schedule_covered,
            "counterfactual_exit_fee_attribution",
            "sourced versioned fee schedule and quote-aligned order-class attribution are required",
        ),
        (
            "entry_fee_source_coverage_percent",
            fee_covered,
            "entry_context.entry_fee_currencies,entry_context.entry_fill_links",
            "fee amount and linked fill currencies are missing or not in the quote currency",
        ),
        (
            "executable_path_source_coverage_percent",
            path_covered,
            "pre_exit_price_path_evidence,price_path_evidence",
            "complete pre-exit identity/path and post-exit path evidence are required",
        ),
        (
            "authoritative_initial_risk_source_coverage_percent",
            initial_risk_covered,
            "entry_context.initial_risk_reference,realized_r_evidence",
            "complete versioned initial-risk reference and linked realized-R evidence are required",
        ),
        (
            "measured_execution_cost_source_coverage_percent",
            measured_cost_covered,
            "measured_slippage_quote,measured_slippage_currency,execution_costs_complete",
            "measured quote-currency slippage and complete execution-cost evidence are required",
        ),
        (
            "timestamped_fx_source_coverage_percent",
            fx_covered,
            "fx_path",
            "sourced timestamp-aligned FX path is unavailable",
        ),
        (
            "independent_settlement_source_coverage_percent",
            reconciled,
            "independent_reconciliation",
            "independently reconciled Paper Exchange execution ledger and evidence epoch are unavailable",
        ),
    ] {
        let status = if matching.is_empty() {
            Unavailable
        } else if covered == matching.len() {
            strategy::ExitPolicyMetricCoverageStatus::Complete
        } else if covered > 0 {
            strategy::ExitPolicyMetricCoverageStatus::Partial
        } else {
            Unavailable
        };
        let value = (!matching.is_empty()).then(|| covered as f64 / matching.len() as f64 * 100.0);
        add_metric(
            &mut metrics,
            key,
            value,
            metric_coverage(
                status,
                covered,
                &source.split(',').collect::<Vec<_>>(),
                (matching.is_empty() || covered < matching.len()).then_some(unavailable_reason),
            ),
        );
    }
    metrics.coverage.insert(
        "complete_replay_input_coverage_percent".to_owned(),
        metric_coverage(
            if matching.is_empty() {
                Unavailable
            } else if complete_samples == matching.len() {
                strategy::ExitPolicyMetricCoverageStatus::Complete
            } else {
                strategy::ExitPolicyMetricCoverageStatus::Partial
            },
            complete_samples,
            &[
                "entry_context",
                "pre_exit_price_path_evidence",
                "price_path_evidence",
            ],
            (!matching.is_empty() && complete_samples < matching.len()).then_some(
                "one or more matching observations lack complete executable replay inputs",
            ),
        ),
    );
    metrics
        .promotion_blockers
        .push(if complete_samples < MIN_OBSERVATIONS {
            "insufficient_complete_executable_path_observations".to_owned()
        } else {
            "candidate_training_or_validation_prerequisite_not_met".to_owned()
        });
    strategy::LearnedExitPolicyAssessment {
        symbol: symbol.to_owned(),
        strategy: strategy_name.to_owned(),
        profile_id: None,
        profile_version: None,
        stop_loss_percent: None,
        take_profit_percent: None,
        activation_percent: None,
        giveback_percent: None,
        minimum_lock_percent: None,
        trailing_activation_percent: None,
        trailing_distance_percent: None,
        breakeven_activation_percent: None,
        maximum_holding_duration_ms: None,
        sample_count: complete_samples,
        training_count: 0,
        validation_count: 0,
        as_of_ms: matching
            .iter()
            .filter_map(|value| value["observed_at_ms"].as_u64())
            .filter(|timestamp| *timestamp <= cutoff_timestamp_ms)
            .max()
            .unwrap_or_default(),
        status: if complete_samples < MIN_OBSERVATIONS {
            "insufficient_evidence".to_owned()
        } else {
            "candidate_rejected".to_owned()
        },
        metrics,
    }
}

fn valid_evidence_identifier(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    })
}

fn fee_schedule_covers(schedule: &Value, timestamp_ms: u64) -> bool {
    let Some(effective_from) = schedule["effective_from_utc_ms"].as_u64() else {
        return false;
    };
    if !schedule["effective_to_utc_ms"].is_null()
        && schedule["effective_to_utc_ms"].as_u64().is_none()
    {
        return false;
    }
    let effective_to = schedule["effective_to_utc_ms"].as_u64();
    effective_from <= timestamp_ms && effective_to.is_none_or(|to| timestamp_ms < to)
}

fn validate_fee_currency_conversion(
    conversion: &Value,
    fee_currency: &str,
    quote_currency: &str,
    timestamp_ms: u64,
) -> Option<f64> {
    let from = conversion["from_currency"].as_str()?;
    let to = conversion["to_currency"].as_str()?;
    let rate_to_quote = conversion["rate_to_quote"]
        .as_f64()
        .filter(|rate| rate.is_finite() && *rate > 0.0)?;
    let effective_from = conversion["effective_from_utc_ms"].as_u64()?;
    if !conversion["effective_to_utc_ms"].is_null()
        && conversion["effective_to_utc_ms"].as_u64().is_none()
    {
        return None;
    }
    let effective_to = conversion["effective_to_utc_ms"].as_u64();
    let source_timestamp = conversion["source_timestamp_utc_ms"].as_u64()?;
    (conversion["schema_version"].as_u64() == Some(1)
        && conversion["source_schema_version"].as_str()
            == Some("paper_exchange_financial_evidence:v1")
        && from == fee_currency
        && to == quote_currency
        && effective_from <= timestamp_ms
        && effective_to.is_none_or(|to| timestamp_ms < to)
        && source_timestamp <= timestamp_ms
        && valid_evidence_identifier(conversion["source_system"].as_str())
        && valid_evidence_identifier(conversion["source_epoch"].as_str())
        && conversion["source_cursor"]
            .as_u64()
            .is_some_and(|cursor| cursor > 0)
        && valid_evidence_identifier(conversion["source_record_id"].as_str()))
    .then_some(rate_to_quote)
}

/// Validate the source contract used to charge counterfactual exit fees.
/// The legacy `counterfactual_exit_fee_rate` is deliberately not consulted:
/// without this versioned attribution it is display-only and cannot enter
/// net replay, modeled fee burden, or policy eligibility.
fn counterfactual_exit_fee_attribution(
    value: &Value,
    symbol: &str,
    quote_currency: &str,
    path: &[Quote],
) -> Option<CounterfactualExitFeeAttribution> {
    let attribution = &value["counterfactual_exit_fee_attribution"];
    if attribution["schema_version"].as_u64() != Some(1) || path.is_empty() {
        return None;
    }

    let mut schedules = BTreeMap::<(String, String), &Value>::new();
    for schedule in attribution["fee_schedules"].as_array()? {
        let schedule_id = schedule["fee_schedule_id"].as_str()?;
        let schedule_version = schedule["fee_schedule_version"].as_str()?;
        let maker_rate = schedule["maker_rate"].as_f64()?;
        let taker_rate = schedule["taker_rate"].as_f64()?;
        let effective_from = schedule["effective_from_utc_ms"].as_u64()?;
        let source_timestamp = schedule["source_timestamp_utc_ms"].as_u64()?;
        if schedule["source_system"].as_str() != Some("paper_exchange_simulator")
            || schedule["source_schema_version"].as_str()
                != Some("paper_exchange_financial_evidence:v1")
            || schedule["symbol"].as_str() != Some(symbol)
            || !valid_evidence_identifier(Some(schedule_id))
            || !valid_evidence_identifier(Some(schedule_version))
            || !valid_evidence_identifier(schedule["source_event_id"].as_str())
            || !valid_evidence_identifier(schedule["source_epoch"].as_str())
            || !valid_evidence_identifier(schedule["source_record_id"].as_str())
            || schedule["source_cursor"]
                .as_u64()
                .is_none_or(|cursor| cursor == 0)
            || source_timestamp == 0
            || !valid_counterfactual_exit_fee_rate(maker_rate)
            || !valid_counterfactual_exit_fee_rate(taker_rate)
            || effective_from == 0
            || (!schedule["effective_to_utc_ms"].is_null()
                && schedule["effective_to_utc_ms"].as_u64().is_none())
            || schedule["effective_to_utc_ms"]
                .as_u64()
                .is_some_and(|effective_to| effective_to <= effective_from)
            || schedules
                .insert(
                    (schedule_id.to_owned(), schedule_version.to_owned()),
                    schedule,
                )
                .is_some()
        {
            return None;
        }
    }
    if schedules.is_empty() {
        return None;
    }
    let fee_fill_matches_schedule = |fill: &Value, timestamp: Option<u64>| {
        let schedule_key = (
            fill["paper_exchange_fee_schedule_id"].as_str(),
            fill["paper_exchange_fee_schedule_version"].as_str(),
        );
        let (Some(schedule_id), Some(schedule_version)) = schedule_key else {
            return false;
        };
        let Some(schedule) = schedules.get(&(schedule_id.to_owned(), schedule_version.to_owned()))
        else {
            return false;
        };
        let fee = fill["fee_amount"].as_f64();
        let paper_fee = fill["paper_exchange_fee_amount"].as_f64();
        let currency = fill["fee_currency"].as_str();
        let paper_currency = fill["paper_exchange_fee_currency"].as_str();
        let class = fill["paper_exchange_fee_order_class"].as_str();
        let rate = match class {
            Some("maker") => schedule["maker_rate"].as_f64(),
            Some("taker") => schedule["taker_rate"].as_f64(),
            _ => None,
        };
        fee.is_some_and(|value| value.is_finite())
            && paper_fee.is_some_and(|value| value.is_finite() && value >= 0.0)
            && fee.zip(paper_fee).is_some_and(|(pm, paper)| {
                (pm.abs() - paper).abs() <= 1e-9 * pm.abs().max(paper).max(1.0)
            })
            && currency == Some(quote_currency)
            && paper_currency == currency
            && rate.is_some_and(valid_counterfactual_exit_fee_rate)
            && timestamp.is_none_or(|timestamp| fee_schedule_covers(schedule, timestamp))
    };
    let entry_fills = value["entry_context"]["entry_fill_links"].as_array()?;
    let exit_fills = value["exit_fill_evidence"]["fills"].as_array()?;
    if entry_fills.is_empty()
        || exit_fills.is_empty()
        || entry_fills
            .iter()
            .any(|fill| !fee_fill_matches_schedule(fill, fill["timestamp_ms"].as_u64()))
        || exit_fills
            .iter()
            .any(|fill| !fee_fill_matches_schedule(fill, None))
    {
        return None;
    }

    let exit_timestamp = value["exit_timestamp_ms"].as_u64()?;
    let exit_ref = &attribution["exit_fee_schedule_ref"];
    let exit_schedule_key = (
        exit_ref["fee_schedule_id"].as_str()?.to_owned(),
        exit_ref["fee_schedule_version"].as_str()?.to_owned(),
    );
    let exit_schedule = *schedules.get(&exit_schedule_key)?;
    let exit_order_class = exit_ref["exit_order_class"].as_str()?;
    let exit_rate = exit_ref["applied_rate"]
        .as_f64()
        .filter(|rate| valid_counterfactual_exit_fee_rate(*rate))?;
    let exit_fee_currency = exit_ref["fee_currency"].as_str()?;
    let exit_expected_rate = match exit_order_class {
        "maker" => exit_schedule["maker_rate"].as_f64()?,
        "taker" => exit_schedule["taker_rate"].as_f64()?,
        _ => return None,
    };
    if exit_ref["timestamp_ms"].as_u64() != Some(exit_timestamp)
        || exit_rate != exit_expected_rate
        || !valid_evidence_identifier(Some(exit_fee_currency))
        || !fee_schedule_covers(exit_schedule, exit_timestamp)
        || schedules
            .values()
            .filter(|schedule| fee_schedule_covers(schedule, exit_timestamp))
            .count()
            != 1
    {
        return None;
    }
    if exit_fee_currency == quote_currency {
        if !exit_ref["fee_currency_conversion"].is_null() {
            return None;
        }
    } else {
        validate_fee_currency_conversion(
            &exit_ref["fee_currency_conversion"],
            exit_fee_currency,
            quote_currency,
            exit_timestamp,
        )?;
    }

    let raw_quotes = ["pre_exit_price_path", "price_path"]
        .into_iter()
        .flat_map(|key| executable_quote_path(&value[key]).unwrap_or_default())
        .collect::<Vec<_>>();
    let mut raw_by_timestamp = BTreeMap::<u64, &Value>::new();
    for quote in &raw_quotes {
        let timestamp = quote["timestamp"].as_u64()?;
        if let Some(previous) = raw_by_timestamp.get(&timestamp)
            && (previous["bid"] != quote["bid"] || previous["ask"] != quote["ask"])
        {
            return None;
        }
        raw_by_timestamp.insert(timestamp, quote);
    }
    if raw_by_timestamp.len() != path.len() {
        return None;
    }

    let quote_observations = attribution["quote_observations"].as_array()?;
    if quote_observations.len() != path.len() {
        return None;
    }
    let mut fx_rate_to_quote_by_timestamp = BTreeMap::new();
    let mut rate_by_timestamp = BTreeMap::new();
    let mut fee_currency_by_timestamp = BTreeMap::new();
    for (quote, source) in path.iter().zip(quote_observations) {
        let (timestamp, schedule_id, schedule_version, order_class, applied_rate, fee_currency) =
            if source.is_array() {
                (
                    source[0].as_u64()?,
                    source[1].as_str()?,
                    source[2].as_str()?,
                    source[3].as_str()?,
                    source[4].as_f64()?,
                    source[5].as_str()?,
                )
            } else {
                (
                    source["timestamp_ms"].as_u64()?,
                    source["fee_schedule_id"].as_str()?,
                    source["fee_schedule_version"].as_str()?,
                    source["exit_order_class"].as_str()?,
                    source["applied_rate"].as_f64()?,
                    source["fee_currency"].as_str()?,
                )
            };
        let raw = raw_by_timestamp.get(&timestamp)?;
        let quote_epoch = raw["epoch"].as_str()?;
        let quote_sequence = raw["sequence"].as_u64()?;
        let source_updated_at = raw["source_updated_at_ms"].as_u64()?;
        let freshness_max_age = raw["freshness_max_age_ms"].as_u64()?;
        let source_system = raw["quote_source_system"].as_str()?;
        let source_epoch = raw["quote_source_epoch"].as_str()?;
        let source_cursor = raw["quote_source_cursor"].as_u64()?;
        let source_record_id = raw["quote_source_record_id"].as_str()?;
        let schedule_key = (schedule_id.to_owned(), schedule_version.to_owned());
        let schedule = *schedules.get(&schedule_key)?;
        let expected_rate = match order_class {
            "maker" => schedule["maker_rate"].as_f64()?,
            "taker" => schedule["taker_rate"].as_f64()?,
            _ => return None,
        };
        let applied_rate = applied_rate
            .is_finite()
            .then_some(applied_rate)
            .filter(|rate| valid_counterfactual_exit_fee_rate(*rate))?;
        if timestamp != quote.timestamp_ms
            || quote_sequence == 0
            || source_updated_at > timestamp
            || timestamp.saturating_sub(source_updated_at) > freshness_max_age
            || freshness_max_age == 0
            || raw["freshness_contract_version"].as_u64() != Some(1)
            || raw["source_fresh"] != Value::Bool(true)
            || !valid_evidence_identifier(Some(quote_epoch))
            || !valid_evidence_identifier(Some(source_system))
            || !valid_evidence_identifier(Some(source_epoch))
            || source_cursor == 0
            || !valid_evidence_identifier(Some(source_record_id))
            || schedule_id != schedule_key.0
            || schedule_version != schedule_key.1
            || applied_rate != expected_rate
            || !fee_schedule_covers(schedule, timestamp)
            || schedules
                .values()
                .filter(|schedule| fee_schedule_covers(schedule, timestamp))
                .count()
                != 1
            || quote.timestamp_ms != timestamp
            || raw["epoch"].as_str().is_none()
            || raw["sequence"].as_u64().is_none()
            || raw["source_updated_at_ms"].as_u64().is_none()
            || raw["source_fresh"].as_bool() != Some(true)
            || raw["freshness_contract_version"].as_u64() != Some(1)
        {
            return None;
        }
        if !valid_evidence_identifier(Some(fee_currency)) {
            return None;
        }
        let conversion = if source.is_array() {
            &Value::Null
        } else {
            &source["fee_currency_conversion"]
        };
        let fx_rate = if fee_currency == quote_currency {
            if !conversion.is_null() {
                return None;
            }
            None
        } else {
            Some(validate_fee_currency_conversion(
                conversion,
                fee_currency,
                quote_currency,
                timestamp,
            )?)
        };
        if fx_rate_to_quote_by_timestamp
            .insert(timestamp, fx_rate)
            .is_some()
        {
            return None;
        }
        rate_by_timestamp.insert(timestamp, applied_rate);
        fee_currency_by_timestamp.insert(timestamp, fee_currency.to_owned());
    }
    Some(CounterfactualExitFeeAttribution {
        rate_by_timestamp,
        fee_currency_by_timestamp,
        quote_currency: quote_currency.to_owned(),
        fx_rate_to_quote_by_timestamp,
    })
}

fn counterfactual_exit_fee_contract_complete(value: &Value) -> bool {
    let Some(symbol) = value["symbol"].as_str() else {
        return false;
    };
    let Some(quote_currency) = symbol.rsplit_once('-').map(|(_, quote)| quote) else {
        return false;
    };
    let quotes = ["pre_exit_price_path", "price_path"]
        .into_iter()
        .flat_map(|key| executable_quote_path(&value[key]).unwrap_or_default())
        .map(|quote| parse_quote(&quote))
        .collect::<Option<Vec<_>>>();
    let Some(mut quotes) = quotes else {
        return false;
    };
    quotes.sort_by_key(|quote| quote.timestamp_ms);
    quotes.dedup_by_key(|quote| quote.timestamp_ms);
    counterfactual_exit_fee_attribution(value, symbol, quote_currency, &quotes).is_some()
}

fn gross_executable_excursion_extrema(
    entry_price: f64,
    quantity: f64,
    direction: f64,
    path: &[Quote],
) -> Option<(f64, f64)> {
    let notional = entry_price * quantity;
    if !notional.is_finite() || notional <= 0.0 || path.is_empty() {
        return None;
    }
    let returns = path
        .iter()
        .map(|quote| {
            let executable_price = if direction > 0.0 {
                quote.bid
            } else {
                quote.ask
            };
            ((executable_price - entry_price) * quantity * direction) / notional * 100.0
        })
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    Some((
        returns.iter().copied().reduce(f64::max)?,
        returns.iter().copied().reduce(f64::min)?,
    ))
}

fn net_executable_excursion_extrema(
    entry_price: f64,
    quantity: f64,
    direction: f64,
    entry_fees_quote: f64,
    path: &[Quote],
    attribution: &CounterfactualExitFeeAttribution,
) -> Option<(f64, f64)> {
    let notional = entry_price * quantity;
    if !notional.is_finite()
        || notional <= 0.0
        || !entry_fees_quote.is_finite()
        || entry_fees_quote < 0.0
        || path.is_empty()
        || attribution.quote_currency.trim().is_empty()
    {
        return None;
    }
    let returns = path
        .iter()
        .map(|quote| {
            let executable_price = if direction > 0.0 {
                quote.bid
            } else {
                quote.ask
            };
            let fee_currency = attribution
                .fee_currency_by_timestamp
                .get(&quote.timestamp_ms)?;
            let fee_rate = attribution.rate_by_timestamp.get(&quote.timestamp_ms)?;
            let fee_in_quote = if fee_currency == &attribution.quote_currency {
                executable_price * quantity * fee_rate
            } else {
                let fx_rate = attribution
                    .fx_rate_to_quote_by_timestamp
                    .get(&quote.timestamp_ms)
                    .copied()
                    .flatten()
                    .filter(|rate| rate.is_finite() && *rate > 0.0)?;
                let notional_in_fee_currency = executable_price * quantity / fx_rate;
                let fee_in_fee_currency = notional_in_fee_currency * fee_rate;
                fee_in_fee_currency * fx_rate
            };
            let net = ((executable_price - entry_price) * quantity * direction
                - entry_fees_quote
                - fee_in_quote)
                / notional
                * 100.0;
            net.is_finite().then_some(net)
        })
        .collect::<Option<Vec<_>>>()?;
    Some((
        returns.iter().copied().reduce(f64::max)?,
        returns.iter().copied().reduce(f64::min)?,
    ))
}

fn gross_only_excursion_for_observation(
    value: &Value,
    symbol: &str,
    strategy_name: &str,
) -> Option<[f64; 2]> {
    if value["symbol"].as_str()? != symbol
        || value["strategy"].as_str()? != strategy_name
        || value["pre_exit_price_path_evidence"]["complete"].as_bool() != Some(true)
        || value["pre_exit_price_path_evidence"]["identity_complete"].as_bool() != Some(true)
    {
        return None;
    }
    let entry = &value["entry_context"];
    let entry_price = entry["average_price"].as_f64()?;
    let quantity = entry["quantity"].as_f64()?;
    let direction = match value["side"].as_str()? {
        "sell" => 1.0,
        "buy" => -1.0,
        _ => return None,
    };
    let raw_path = executable_quote_path(&value["pre_exit_price_path"])?;
    let path = raw_path
        .iter()
        .map(parse_quote)
        .collect::<Option<Vec<_>>>()?;
    let (mfe, mae) = gross_executable_excursion_extrema(entry_price, quantity, direction, &path)?;
    Some([mfe, mae])
}

fn parse_observation(
    value: &Value,
    symbol: &str,
    strategy_name: &str,
    cutoff_timestamp_ms: u64,
) -> Option<ReplayObservation> {
    if value["symbol"].as_str()? != symbol || value["strategy"].as_str()? != strategy_name {
        return None;
    }
    if !matches!(
        value["reason"].as_str()?,
        "profit-protection" | "take-profit" | "stop-loss"
    ) {
        return None;
    }
    let exit_timestamp_ms = value["exit_timestamp_ms"].as_u64()?;
    let observed_at_ms = value["observed_at_ms"].as_u64()?;
    let event_id = value["event_id"].as_str()?.trim();
    let horizon_ms = value["horizon_ms"].as_u64()?;
    if exit_timestamp_ms == 0
        || event_id.is_empty()
        || horizon_ms == 0
        || observed_at_ms > cutoff_timestamp_ms
        || observed_at_ms < exit_timestamp_ms
    {
        return None;
    }
    let entry = &value["entry_context"];
    let quote_currency = symbol.rsplit_once('-').map(|(_, quote)| quote.to_owned());
    let entry_price = entry["average_price"].as_f64()?;
    let quantity = entry["quantity"].as_f64()?;
    let entry_fees = entry_fee_quote(entry, quote_currency.as_deref())?;
    let direction = match value["side"].as_str()? {
        "sell" => 1.0,
        "buy" => -1.0,
        _ => return None,
    };
    let stop_loss_percent = value["effective_stop_loss_percent"].as_f64()?;
    let take_profit_percent = value["effective_take_profit_percent"].as_f64()?;
    // strategy_decision is copied from the entry audit and preserves the
    // actual parameters applied to this position. Never infer a baseline
    // from current defaults: old/open-position policy can differ.
    let parameters = &value["strategy_decision"]["strategy_parameters"];
    let baseline = ProfitProtectionParameters {
        stop_loss_percent,
        take_profit_percent,
        activation_percent: parameters[ACTIVATION_KEY].as_f64()?,
        giveback_percent: parameters[GIVEBACK_KEY].as_f64()?,
        minimum_lock_percent: parameters[MINIMUM_LOCK_KEY].as_f64()?,
        trailing_activation_percent: parameters[TRAILING_ACTIVATION_KEY].as_f64(),
        trailing_distance_percent: parameters[TRAILING_DISTANCE_KEY].as_f64(),
        breakeven_activation_percent: parameters[BREAKEVEN_ACTIVATION_KEY].as_f64(),
    };
    if ![
        entry_price,
        quantity,
        entry_fees,
        stop_loss_percent,
        take_profit_percent,
        baseline.activation_percent,
        baseline.giveback_percent,
        baseline.minimum_lock_percent,
    ]
    .iter()
    .all(|number| number.is_finite())
        || entry_price <= 0.0
        || quantity <= 0.0
        || entry_fees < 0.0
    {
        return None;
    }
    let risk_reference = &entry["initial_risk_reference"];
    let initial_risk_currency = risk_reference["quote_currency"].as_str().map(str::to_owned);
    let entry_fill_ids = entry["entry_fill_ids"].as_array();
    let risk_fill_ids = risk_reference["fill_ids"].as_array();
    let realized_r_fill_ids = value["realized_r_evidence"]["initial_risk_fill_ids"].as_array();
    let initial_risk_quote = risk_reference["quote_amount"]
        .as_f64()
        .filter(|risk| risk.is_finite() && *risk > 0.0)
        .filter(|_| {
            risk_reference["schema_version"].as_u64() == Some(1)
                && risk_reference["status"].as_str() == Some("complete")
                && risk_reference["quote_currency"].as_str() == quote_currency.as_deref()
                && entry["entry_fill_costs_complete"].as_bool() == Some(true)
                && value["pre_exit_price_path_evidence"]["identity_complete"].as_bool()
                    == Some(true)
                && value["realized_r_evidence"]["status"].as_str() == Some("complete")
                && value["realized_r_evidence"]["schema_version"].as_u64() == Some(1)
                && value["realized_r_evidence"]["quote_currency"].as_str()
                    == quote_currency.as_deref()
                && value["realized_r_evidence"]["expected_quote_currency"].as_str()
                    == quote_currency.as_deref()
                && value["realized_r_evidence"]["initial_risk_quote_amount"].as_f64()
                    == risk_reference["quote_amount"].as_f64()
                && realized_r_fill_ids
                    .is_some_and(|ids| risk_fill_ids.is_some_and(|risk_ids| ids == risk_ids))
                && entry_fill_ids.is_some_and(|ids| !ids.is_empty())
                && risk_fill_ids.is_some_and(|ids| {
                    entry_fill_ids.is_some_and(|entry_ids| {
                        ids.len() == entry_ids.len() && ids.iter().all(|id| entry_ids.contains(id))
                    })
                })
        });
    let independent_reconciliation = &value["independent_reconciliation"];
    let independent_reconciliation_complete = independent_reconciliation["status"].as_str()
        == Some("reconciled")
        && independent_reconciliation["source"].as_str() == Some("paper_exchange_execution_ledger")
        && independent_reconciliation["evidence_epoch_id"]
            .as_str()
            .is_some_and(|epoch| !epoch.trim().is_empty());
    let post_path_evidence = &value["price_path_evidence"];
    let pre_path_evidence = &value["pre_exit_price_path_evidence"];
    if post_path_evidence["complete"].as_bool() != Some(true)
        || pre_path_evidence["complete"].as_bool() != Some(true)
    {
        return None;
    }
    let pre_exit_path = executable_quote_path(&value["pre_exit_price_path"])?
        .iter()
        .map(parse_quote)
        .collect::<Option<Vec<_>>>()?;
    if pre_exit_path.is_empty()
        || pre_exit_path
            .iter()
            .any(|quote| quote.timestamp_ms > exit_timestamp_ms)
    {
        return None;
    }
    let post_exit_path = executable_quote_path(&value["price_path"])?
        .iter()
        .map(parse_quote)
        .collect::<Option<Vec<_>>>()?;
    if post_exit_path
        .iter()
        .any(|quote| quote.timestamp_ms < exit_timestamp_ms)
    {
        return None;
    }
    if !has_horizon_coverage(&post_exit_path, exit_timestamp_ms, horizon_ms) {
        return None;
    }
    let mut path = pre_exit_path.clone();
    path.extend(post_exit_path);
    path.sort_by_key(|quote| quote.timestamp_ms);
    path.dedup_by_key(|quote| quote.timestamp_ms);
    if path.len() < 2 || path.last()?.timestamp_ms > observed_at_ms {
        return None;
    }
    let quote_currency_str = quote_currency.as_deref()?;
    let fee_attribution =
        counterfactual_exit_fee_attribution(value, symbol, quote_currency_str, &path)?;
    if fee_attribution.rate_by_timestamp.is_empty()
        || fee_attribution
            .rate_by_timestamp
            .values()
            .any(|rate| !valid_counterfactual_exit_fee_rate(*rate))
        || !valid_exit_policy_inputs(stop_loss_percent, take_profit_percent, baseline)
    {
        return None;
    }
    // Gross and net MFE/MAE use only the path before the actual close. The
    // post-exit path is reserved for counterfactual holding replay.
    let (gross_executable_mfe_percent, gross_executable_mae_percent) =
        gross_executable_excursion_extrema(entry_price, quantity, direction, &pre_exit_path)?;
    let (net_executable_mfe_percent, net_executable_mae_percent) =
        net_executable_excursion_extrema(
            entry_price,
            quantity,
            direction,
            entry_fees,
            &pre_exit_path,
            &fee_attribution,
        )?;
    let actual_exit_net_return_percent = value["exit_return_percent"].as_f64()?;
    if !actual_exit_net_return_percent.is_finite() {
        return None;
    }
    let entry_timestamp_ms = entry["timestamp_ms"].as_u64();
    let maximum_holding_duration_days =
        value["strategy_decision"]["strategy_parameters"]["maximum_hold_days"]
            .as_f64()
            .filter(|days| valid_maximum_hold_days(strategy_name, *days));
    let provenance_profile_id = value["exit_parameter_provenance"]["profile_id"]
        .as_str()
        .map(str::to_owned);
    let decision_profile_id = entry["strategy_decision"]["learning_profile_id"]
        .as_str()
        .map(str::to_owned);
    let provenance_profile_version = value["exit_parameter_provenance"]["profile_version"].as_u64();
    let decision_profile_version = entry["strategy_decision"]["learning_profile_version"].as_u64();
    if provenance_profile_id
        .as_ref()
        .zip(decision_profile_id.as_ref())
        .is_some_and(|(provenance, decision)| provenance != decision)
        || provenance_profile_version
            .zip(decision_profile_version)
            .is_some_and(|(provenance, decision)| provenance != decision)
    {
        return None;
    }
    let (initial_entry_notional_quote, entry_fill_timing) =
        entry_fill_timing(entry, Some(quote_currency_str))
            .map(|(notional, fills)| (Some(notional), Some(fills)))
            .unwrap_or((None, None));
    Some(ReplayObservation {
        event_id: event_id.to_owned(),
        exit_timestamp_ms,
        observed_at_ms,
        direction,
        entry_price,
        quantity,
        entry_fees,
        initial_risk_quote,
        initial_risk_currency,
        quote_currency,
        market_regime: value["market_regime"]
            .as_str()
            .or_else(|| value["strategy_decision"]["market_regime"].as_str())
            .map(str::to_owned),
        entry_timestamp_ms,
        maximum_holding_duration_days,
        profile_id: provenance_profile_id.or(decision_profile_id),
        profile_version: provenance_profile_version.or(decision_profile_version),
        initial_entry_notional_quote,
        entry_fill_timing,
        execution_costs_complete: entry["entry_fill_costs_complete"].as_bool() == Some(true)
            && value["exit_fill_evidence"]["fees_complete"].as_bool() == Some(true)
            && value["realized_r_evidence"]["fee_coverage_complete"].as_bool() == Some(true),
        counterfactual_exit_fee_attribution: fee_attribution,
        gross_executable_mfe_percent,
        gross_executable_mae_percent,
        net_executable_mfe_percent,
        net_executable_mae_percent,
        measured_slippage_quote: None,
        measured_slippage_currency: None,
        independent_reconciliation_complete,
        stop_loss_percent,
        take_profit_percent,
        baseline,
        actual_exit_net_return_percent,
        exit_reason: value["reason"].as_str()?.to_owned(),
        path,
    })
}

fn maximum_hold_days_spec(strategy_name: &str) -> Option<(f64, f64, f64, f64)> {
    let strategy = strategy::Strategy::from_name(strategy_name)?;
    let spec = strategy
        .parameter_specs()
        .into_iter()
        .find(|spec| spec.key.as_str() == "maximum_hold_days")?;
    Some((spec.default, spec.min, spec.max, spec.step))
}

fn valid_maximum_hold_days(strategy_name: &str, days: f64) -> bool {
    let Some((_, min, max, step)) = maximum_hold_days_spec(strategy_name) else {
        return false;
    };
    if !days.is_finite() || days < min || days > max || step <= 0.0 {
        return false;
    }
    let step_index = (days - min) / step;
    (step_index - step_index.round()).abs() <= 1e-7
}

fn hold_duration_days_to_ms(days: f64) -> Option<u64> {
    if !days.is_finite() || days <= 0.0 {
        return None;
    }
    let millis = days * MS_PER_DAY;
    (millis.is_finite() && millis > 0.0 && millis <= u64::MAX as f64).then(|| millis.ceil() as u64)
}

fn entry_fee_quote(entry: &Value, quote_currency: Option<&str>) -> Option<f64> {
    let quote_currency = quote_currency?;
    if entry["entry_fill_costs_complete"].as_bool() != Some(true) {
        return None;
    }
    let fee_currencies = entry["entry_fee_currencies"].as_array()?;
    let fill_ids = entry["entry_fill_ids"].as_array()?;
    let fill_links = entry["entry_fill_links"].as_array()?;
    let fill_id_strings = fill_ids
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let linked_fill_ids = fill_links
        .iter()
        .filter_map(|fill| fill["fill_id"].as_str())
        .collect::<Vec<_>>();
    let linked_fee_total = fill_links
        .iter()
        .filter_map(|fill| fill["fee_amount"].as_f64())
        .sum::<f64>();
    let reported_fees = entry["fees"].as_f64()?;
    if fee_currencies.is_empty()
        || fill_ids.is_empty()
        || fill_links.len() != fill_ids.len()
        || fee_currencies
            .iter()
            .any(|currency| currency.as_str() != Some(quote_currency))
        || fill_id_strings.len() != fill_ids.len()
        || fill_id_strings
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != fill_ids.len()
        || linked_fill_ids.len() != fill_links.len()
        || linked_fill_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != fill_links.len()
        || fill_id_strings
            .iter()
            .any(|id| !linked_fill_ids.contains(id))
        || !reported_fees.is_finite()
        || reported_fees < 0.0
        || !linked_fee_total.is_finite()
        || (linked_fee_total - reported_fees).abs() > 1e-8_f64.max(reported_fees.abs() * 1e-8)
        || fill_links.iter().any(|fill| {
            fill["fee_known"].as_bool() != Some(true)
                || fill["fee_currency"].as_str() != Some(quote_currency)
                || fill["fee_amount"]
                    .as_f64()
                    .is_none_or(|amount| !amount.is_finite() || amount < 0.0)
        })
    {
        return None;
    }
    Some(reported_fees)
}

fn entry_fill_timing(
    entry: &Value,
    quote_currency: Option<&str>,
) -> Option<(f64, Vec<EntryFillTiming>)> {
    let quote_currency = quote_currency?;
    let initial_notional = entry["initial_entry_notional"]
        .as_f64()
        .filter(|amount| amount.is_finite() && *amount > 0.0)?;
    let links = entry["entry_fill_links"].as_array()?;
    let ids = entry["entry_fill_ids"].as_array()?;
    if entry["entry_fill_costs_complete"].as_bool() != Some(true)
        || links.is_empty()
        || links.len() != ids.len()
    {
        return None;
    }
    let mut seen_ids = BTreeSet::new();
    let mut fills = Vec::with_capacity(links.len());
    let mut total_notional = 0.0;
    for link in links {
        let fill_id = link["fill_id"].as_str()?.trim();
        let timestamp_ms = link["timestamp_ms"].as_u64()?;
        let quantity = link["quantity"].as_f64()?;
        let price = link["execution_price"].as_f64()?;
        if fill_id.is_empty()
            || !seen_ids.insert(fill_id.to_owned())
            || !ids.iter().any(|id| id.as_str() == Some(fill_id))
            || timestamp_ms == 0
            || !quantity.is_finite()
            || quantity <= 0.0
            || !price.is_finite()
            || price <= 0.0
            || link["fee_currency"].as_str() != Some(quote_currency)
            || link["fee_known"].as_bool() != Some(true)
            || link["fee_amount"]
                .as_f64()
                .is_none_or(|fee| !fee.is_finite() || fee < 0.0)
        {
            return None;
        }
        let notional_quote = quantity * price;
        if !notional_quote.is_finite() || notional_quote <= 0.0 {
            return None;
        }
        total_notional += notional_quote;
        fills.push(EntryFillTiming {
            timestamp_ms,
            notional_quote,
        });
    }
    if seen_ids.len() != ids.len()
        || ids
            .iter()
            .any(|id| id.as_str().is_none_or(|id| !seen_ids.contains(id)))
        || !total_notional.is_finite()
        || (total_notional - initial_notional).abs() > 1e-8_f64.max(initial_notional.abs() * 1e-8)
    {
        return None;
    }
    Some((initial_notional, fills))
}

fn capital_efficiency_sample(
    sample: &ReplayObservation,
    outcome: ReplayOutcome,
) -> Option<(f64, f64)> {
    let initial_notional = sample.initial_entry_notional_quote?;
    let fills = sample.entry_fill_timing.as_ref()?;
    if sample.entry_price * sample.quantity <= 0.0
        || (sample.entry_price * sample.quantity - initial_notional).abs()
            > 1e-8_f64.max(initial_notional.abs() * 1e-8)
        || fills
            .iter()
            .any(|fill| fill.timestamp_ms == 0 || fill.timestamp_ms > outcome.exit_timestamp_ms)
    {
        return None;
    }
    let capital_quote_days = fills
        .iter()
        .map(|fill| {
            fill.notional_quote * outcome.exit_timestamp_ms.saturating_sub(fill.timestamp_ms) as f64
                / MS_PER_DAY
        })
        .sum::<f64>();
    let net_pnl_quote = outcome.net_return_percent / 100.0 * initial_notional;
    (capital_quote_days.is_finite() && capital_quote_days > 0.0 && net_pnl_quote.is_finite())
        .then_some((net_pnl_quote, capital_quote_days))
}

fn valid_exit_policy_inputs(
    stop_loss_percent: f64,
    take_profit_percent: f64,
    baseline: ProfitProtectionParameters,
) -> bool {
    stop_loss_percent.is_finite()
        && stop_loss_percent > 0.0
        && take_profit_percent.is_finite()
        && take_profit_percent > 0.0
        && valid_policy_parameters(baseline)
}

fn valid_counterfactual_exit_fee_rate(rate: f64) -> bool {
    rate.is_finite() && (0.0..=MAX_COUNTERFACTUAL_EXIT_FEE_RATE).contains(&rate)
}

fn has_horizon_coverage(path: &[Quote], exit_timestamp_ms: u64, horizon_ms: u64) -> bool {
    if path.len() < MIN_POST_EXIT_PATH_SAMPLES || horizon_ms == 0 {
        return false;
    }
    let mut timestamps = path
        .iter()
        .filter(|quote| quote.timestamp_ms >= exit_timestamp_ms)
        .map(|quote| quote.timestamp_ms)
        .collect::<Vec<_>>();
    timestamps.sort_unstable();
    timestamps.dedup();
    let (Some(first), Some(last)) = (timestamps.first().copied(), timestamps.last().copied())
    else {
        return false;
    };
    let coverage_required = horizon_ms.saturating_mul(MIN_HORIZON_COVERAGE_PERCENT) / 100;
    let edge_gap_limit = horizon_ms.saturating_mul(MAX_POST_EXIT_PATH_GAP_PERCENT) / 100;
    if first.saturating_sub(exit_timestamp_ms) > edge_gap_limit
        || last.saturating_sub(exit_timestamp_ms) < coverage_required
    {
        return false;
    }
    timestamps
        .windows(2)
        .all(|pair| pair[1].saturating_sub(pair[0]) <= edge_gap_limit)
}

fn parse_quote(value: &Value) -> Option<Quote> {
    let quote = Quote {
        timestamp_ms: value["timestamp"].as_u64()?,
        bid: value["bid"].as_f64()?,
        ask: value["ask"].as_f64()?,
    };
    (quote.timestamp_ms > 0
        && quote.bid.is_finite()
        && quote.ask.is_finite()
        && quote.bid > 0.0
        && quote.ask >= quote.bid)
        .then_some(quote)
}

fn is_supported_quote_path(value: &Value) -> bool {
    value.as_array().is_some() || value["encoding"].as_str() == Some("executable_quote_path:v1")
}

fn executable_quote_path(value: &Value) -> Option<Vec<Value>> {
    if let Some(quotes) = value.as_array() {
        return Some(quotes.clone());
    }
    if value["encoding"].as_str() != Some("executable_quote_path:v1") {
        return None;
    }
    let sources = value["sources"].as_array()?;
    value["quotes"]
        .as_array()?
        .iter()
        .map(|row| {
            let values = row.as_array()?;
            if values.len() != 9 {
                return None;
            }
            let source = sources.get(values[6].as_u64()? as usize)?;
            Some(serde_json::json!({
                "timestamp":values[0],
                "bid":values[1],
                "ask":values[2],
                "sequence":values[3],
                "source_updated_at_ms":values[4],
                "source_fresh":values[5],
                "epoch":source["epoch"],
                "freshness_contract_version":source["freshness_contract_version"],
                "freshness_max_age_ms":source["freshness_max_age_ms"],
                "quote_source_system":source["quote_source_system"],
                "quote_source_epoch":source["quote_source_epoch"],
                "quote_source_cursor":values[7],
                "quote_source_record_id":values[8],
            }))
        })
        .collect()
}

fn replay_net_return(
    sample: &ReplayObservation,
    parameters: ProfitProtectionParameters,
) -> Option<f64> {
    replay_outcome(sample, parameters).map(|outcome| outcome.net_return_percent)
}

fn replay_net_return_with_hold_duration(
    sample: &ReplayObservation,
    parameters: ProfitProtectionParameters,
    maximum_holding_duration_days: Option<f64>,
) -> Option<f64> {
    replay_outcome_with_hold_duration(sample, parameters, maximum_holding_duration_days)
        .map(|outcome| outcome.net_return_percent)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplayExitReason {
    StopLoss,
    BreakevenProtection,
    TrailingStop,
    ProfitProtection,
    MaximumHold,
    TakeProfit,
    Horizon,
}

#[derive(Clone, Copy, Debug)]
struct ReplayOutcome {
    net_return_percent: f64,
    exit_price: f64,
    exit_timestamp_ms: u64,
    #[allow(dead_code)]
    exit_reason: ReplayExitReason,
}

fn metric_coverage(
    status: strategy::ExitPolicyMetricCoverageStatus,
    sample_count: usize,
    source_fields: &[&str],
    reason: Option<&str>,
) -> strategy::ExitPolicyMetricCoverage {
    strategy::ExitPolicyMetricCoverage {
        status,
        sample_count,
        source_fields: source_fields
            .iter()
            .map(|field| (*field).to_owned())
            .collect(),
        reason: reason.map(str::to_owned),
    }
}

fn max_drawdown(returns: &[f64]) -> f64 {
    let (mut cumulative, mut peak, mut drawdown) = (0.0_f64, 0.0_f64, 0.0_f64);
    for value in returns {
        cumulative += value;
        peak = peak.max(cumulative);
        drawdown = drawdown.max(peak - cumulative);
    }
    drawdown
}

fn add_metric(
    report: &mut strategy::ExitPolicyMetricReport,
    key: &str,
    value: Option<f64>,
    coverage: strategy::ExitPolicyMetricCoverage,
) {
    if let Some(value) = value.filter(|value| value.is_finite()) {
        report.values.insert(key.to_owned(), value);
    }
    report.coverage.insert(key.to_owned(), coverage);
}

fn exit_policy_metrics(
    paired: &[(&ReplayObservation, f64, f64, ReplayOutcome)],
    strategy_name: &str,
    selected_hold_duration_days: Option<f64>,
) -> strategy::ExitPolicyMetricReport {
    use strategy::ExitPolicyMetricCoverageStatus::{Complete, Partial, Unavailable};

    let returns = paired.iter().map(|row| row.2).collect::<Vec<_>>();
    let count = returns.len();
    let mean = (!returns.is_empty()).then(|| returns.iter().sum::<f64>() / count as f64);
    let winners = returns
        .iter()
        .copied()
        .filter(|value| *value > 0.0)
        .collect::<Vec<_>>();
    let losers = returns
        .iter()
        .copied()
        .filter(|value| *value < 0.0)
        .collect::<Vec<_>>();
    let gross_wins = winners.iter().sum::<f64>();
    let gross_losses = losers.iter().map(|value| value.abs()).sum::<f64>();
    let average_winner = (!winners.is_empty()).then(|| gross_wins / winners.len() as f64);
    let average_loser = (!losers.is_empty()).then(|| -gross_losses / losers.len() as f64);
    let profit_factor = (gross_losses > 0.0).then(|| gross_wins / gross_losses);
    let payoff_ratio = average_winner
        .zip(average_loser)
        .and_then(|(win, loss)| (loss.abs() > 0.0).then(|| win / loss.abs()));
    let variance = if count > 1 {
        let mean_value = mean.unwrap_or_default();
        Some(
            returns
                .iter()
                .map(|value| (value - mean_value).powi(2))
                .sum::<f64>()
                / (count - 1) as f64,
        )
    } else {
        None
    };
    let risk_adjusted = mean
        .zip(variance)
        .and_then(|(mean, variance)| (variance > 0.0).then(|| mean / variance.sqrt()));
    let drawdown = (!returns.is_empty()).then(|| max_drawdown(&returns));
    let actual_winners = paired
        .iter()
        .filter(|row| row.0.actual_exit_net_return_percent > 0.0);
    let actual_winner_count = actual_winners.clone().count();
    let winner_preservation = (actual_winner_count > 0).then(|| {
        actual_winners.filter(|row| row.2 > 0.0).count() as f64 / actual_winner_count as f64
    });
    let actual_losers = paired
        .iter()
        .filter(|row| row.0.actual_exit_net_return_percent < 0.0);
    let actual_loser_count = actual_losers.clone().count();
    let loser_recovery = (actual_loser_count > 0).then(|| {
        actual_losers
            .filter(|row| row.2 > row.0.actual_exit_net_return_percent)
            .count() as f64
            / actual_loser_count as f64
    });
    let risk_covered = paired
        .iter()
        .filter(|row| row.0.initial_risk_quote.is_some())
        .count();
    let r_values = paired
        .iter()
        .filter_map(|row| {
            let risk = row.0.initial_risk_quote?;
            let quote_currency = row.0.quote_currency.as_deref()?;
            (row.0.initial_risk_currency.as_deref() == Some(quote_currency))
                .then_some((row.2 / 100.0) * row.0.entry_price * row.0.quantity / risk)
        })
        .collect::<Vec<_>>();
    let expectancy_r = (risk_covered == count && count > 0 && r_values.len() == count)
        .then(|| r_values.iter().sum::<f64>() / count as f64);
    let duration_values = paired
        .iter()
        .filter_map(|row| {
            let entry = row.0.entry_timestamp_ms?;
            (entry <= row.3.exit_timestamp_ms)
                .then_some(row.3.exit_timestamp_ms.saturating_sub(entry) as f64)
        })
        .collect::<Vec<_>>();
    let mean_duration_ms = (duration_values.len() == count && count > 0)
        .then(|| duration_values.iter().sum::<f64>() / count as f64);
    let duration_supported = maximum_hold_days_spec(strategy_name).is_some();
    let duration_covered = paired
        .iter()
        .filter(|row| {
            row.0.maximum_holding_duration_days.is_some()
                && row.0.entry_timestamp_ms.is_some()
                && selected_hold_duration_days.is_some()
        })
        .count();
    let duration_coverage_complete = duration_supported && duration_covered == count && count > 0;
    let maximum_holding_duration_ms = duration_coverage_complete
        .then(|| selected_hold_duration_days.and_then(hold_duration_days_to_ms))
        .flatten();
    let extension_values = paired
        .iter()
        .map(|row| {
            row.3
                .exit_timestamp_ms
                .saturating_sub(row.0.exit_timestamp_ms) as f64
        })
        .collect::<Vec<_>>();
    let mean_extension_ms = (!extension_values.is_empty())
        .then(|| extension_values.iter().sum::<f64>() / extension_values.len() as f64);
    let fee_burdens = paired
        .iter()
        .filter_map(|row| {
            let notional = row.0.entry_price * row.0.quantity;
            let modeled_exit_fee =
                counterfactual_exit_fee_quote(row.0, row.3.exit_price, row.3.exit_timestamp_ms)?;
            (notional.is_finite() && notional > 0.0)
                .then_some((row.0.entry_fees + modeled_exit_fee) / notional * 100.0)
        })
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let modeled_fee_percent = (!fee_burdens.is_empty())
        .then(|| fee_burdens.iter().sum::<f64>() / fee_burdens.len() as f64);
    let capital_efficiency_rows = paired
        .iter()
        .filter_map(|row| capital_efficiency_sample(row.0, row.3))
        .collect::<Vec<_>>();
    let capital_efficiency_covered = capital_efficiency_rows.len();
    let capital_efficiency = (capital_efficiency_covered == count && count > 0).then(|| {
        let net_quote_pnl = capital_efficiency_rows
            .iter()
            .map(|(pnl, _)| pnl)
            .sum::<f64>();
        let capital_quote_days = capital_efficiency_rows
            .iter()
            .map(|(_, capital_days)| capital_days)
            .sum::<f64>();
        net_quote_pnl / capital_quote_days
    });
    let measured_cost_covered = paired
        .iter()
        .filter(|row| {
            row.0.execution_costs_complete
                && row.0.measured_slippage_quote.is_some()
                && row.0.measured_slippage_currency == row.0.quote_currency
        })
        .count();
    let measured_slippage_percent = paired
        .iter()
        .filter_map(|row| {
            if !row.0.execution_costs_complete {
                return None;
            }
            if row.0.measured_slippage_currency != row.0.quote_currency {
                return None;
            }
            let slippage = row.0.measured_slippage_quote?;
            let notional = row.0.entry_price * row.0.quantity;
            (notional.is_finite() && notional > 0.0).then_some(slippage / notional * 100.0)
        })
        .collect::<Vec<_>>();
    let mean_measured_slippage_percent = (!measured_slippage_percent.is_empty()).then(|| {
        measured_slippage_percent.iter().sum::<f64>() / measured_slippage_percent.len() as f64
    });
    let time_normalized_return = mean
        .zip(mean_duration_ms)
        .and_then(|(mean, duration)| (duration > 0.0).then(|| mean / (duration / 3_600_000.0)));

    let mut report = strategy::ExitPolicyMetricReport::default();
    let replay_sources = [
        "price_path.bid",
        "price_path.ask",
        "entry_context.average_price",
        "entry_context.quantity",
        "entry_context.fees",
        "counterfactual_exit_fee_attribution",
    ];
    add_metric(
        &mut report,
        "net_expectancy_percent",
        mean,
        metric_coverage(Complete, count, &replay_sources, None),
    );
    for (key, values, sources) in [
        (
            "gross_executable_mfe_percent",
            paired
                .iter()
                .map(|row| row.0.gross_executable_mfe_percent)
                .collect::<Vec<_>>(),
            vec![
                "pre_exit_price_path.bid",
                "pre_exit_price_path.ask",
                "entry_context.average_price",
            ],
        ),
        (
            "gross_executable_mae_percent",
            paired
                .iter()
                .map(|row| row.0.gross_executable_mae_percent)
                .collect::<Vec<_>>(),
            vec![
                "pre_exit_price_path.bid",
                "pre_exit_price_path.ask",
                "entry_context.average_price",
            ],
        ),
        (
            "net_executable_mfe_percent",
            paired
                .iter()
                .map(|row| row.0.net_executable_mfe_percent)
                .collect::<Vec<_>>(),
            vec![
                "pre_exit_price_path",
                "entry_context.entry_fill_links",
                "counterfactual_exit_fee_attribution",
            ],
        ),
        (
            "net_executable_mae_percent",
            paired
                .iter()
                .map(|row| row.0.net_executable_mae_percent)
                .collect::<Vec<_>>(),
            vec![
                "pre_exit_price_path",
                "entry_context.entry_fill_links",
                "counterfactual_exit_fee_attribution",
            ],
        ),
    ] {
        let mean = (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64);
        add_metric(
            &mut report,
            key,
            mean,
            metric_coverage(
                if mean.is_some() {
                    Complete
                } else {
                    Unavailable
                },
                values.len(),
                &sources,
                mean.is_none()
                    .then_some("at least one complete chronological holdout pair is required"),
            ),
        );
    }
    add_metric(
        &mut report,
        "profit_factor",
        profit_factor,
        metric_coverage(
            if profit_factor.is_some() {
                Complete
            } else {
                Unavailable
            },
            count,
            &["replayed_net_return_percent"],
            profit_factor
                .is_none()
                .then_some("no replayed losing outcomes"),
        ),
    );
    add_metric(
        &mut report,
        "payoff_ratio",
        payoff_ratio,
        metric_coverage(
            if payoff_ratio.is_some() {
                Complete
            } else {
                Unavailable
            },
            count,
            &["replayed_net_return_percent"],
            payoff_ratio
                .is_none()
                .then_some("winner and loser observations are both required"),
        ),
    );
    add_metric(
        &mut report,
        "average_winner_percent",
        average_winner,
        metric_coverage(
            if average_winner.is_some() {
                Complete
            } else {
                Unavailable
            },
            winners.len(),
            &["replayed_net_return_percent"],
            average_winner
                .is_none()
                .then_some("no replayed winning outcomes"),
        ),
    );
    add_metric(
        &mut report,
        "average_loser_percent",
        average_loser,
        metric_coverage(
            if average_loser.is_some() {
                Complete
            } else {
                Unavailable
            },
            losers.len(),
            &["replayed_net_return_percent"],
            average_loser
                .is_none()
                .then_some("no replayed losing outcomes"),
        ),
    );
    add_metric(
        &mut report,
        "winner_preservation_rate",
        winner_preservation,
        metric_coverage(
            if winner_preservation.is_some() {
                Complete
            } else {
                Unavailable
            },
            actual_winner_count,
            &["exit_return_percent", "replayed_net_return_percent"],
            winner_preservation
                .is_none()
                .then_some("no captured actual winners"),
        ),
    );
    add_metric(
        &mut report,
        "loser_recovery_rate",
        loser_recovery,
        metric_coverage(
            if loser_recovery.is_some() {
                Complete
            } else {
                Unavailable
            },
            actual_loser_count,
            &["exit_return_percent", "replayed_net_return_percent"],
            loser_recovery
                .is_none()
                .then_some("no captured actual losers"),
        ),
    );
    add_metric(
        &mut report,
        "max_trade_sequence_drawdown_percent_points",
        drawdown,
        metric_coverage(
            Complete,
            count,
            &["replayed_net_return_percent", "exit_timestamp_ms"],
            None,
        ),
    );
    add_metric(
        &mut report,
        "mean_to_sample_standard_deviation",
        risk_adjusted,
        metric_coverage(
            if risk_adjusted.is_some() {
                Complete
            } else {
                Unavailable
            },
            count,
            &["replayed_net_return_percent"],
            risk_adjusted
                .is_none()
                .then_some("at least two non-constant replay returns are required"),
        ),
    );
    add_metric(
        &mut report,
        "expectancy_r",
        expectancy_r,
        metric_coverage(
            if expectancy_r.is_some() {
                Complete
            } else {
                Unavailable
            },
            risk_covered,
            &[
                "entry_context.initial_risk_reference.schema_version",
                "entry_context.initial_risk_reference.status",
                "entry_context.initial_risk_reference.quote_amount",
                "entry_context.initial_risk_reference.quote_currency",
                "entry_context.initial_risk_reference.fill_ids",
                "replayed_net_return_percent",
            ],
            expectancy_r.is_none().then_some(
                "authoritative initial risk in the position quote currency is unavailable",
            ),
        ),
    );
    add_metric(&mut report, "mean_position_duration_ms", mean_duration_ms, metric_coverage(if mean_duration_ms.is_some() { Complete } else { Unavailable }, duration_values.len(), &["entry_context.timestamp_ms", "candidate_replay.exit_timestamp_ms"], mean_duration_ms.is_none().then_some("entry timestamp is unavailable; post-exit horizon is not a substitute for holding duration")));
    add_metric(
        &mut report,
        "maximum_holding_duration_ms",
        maximum_holding_duration_ms.map(|duration| duration as f64),
        metric_coverage(
            if !duration_coverage_complete {
                Unavailable
            } else {
                Complete
            },
            duration_covered,
            &[
                "strategy_decision.strategy_parameters.maximum_hold_days",
                "entry_context.timestamp_ms",
                "pre_exit_price_path_evidence.complete",
                "price_path_evidence.complete",
            ],
            if !duration_supported {
                Some("this strategy has no catalog maximum_hold_days parameter")
            } else if duration_covered != count {
                Some(
                    "captured profile duration and entry timestamp must be complete for every replay sample",
                )
            } else {
                None
            },
        ),
    );
    add_metric(
        &mut report,
        "mean_post_exit_extension_ms",
        mean_extension_ms,
        metric_coverage(
            Complete,
            count,
            &["exit_timestamp_ms", "price_path.timestamp"],
            None,
        ),
    );
    add_metric(
        &mut report,
        "modeled_fee_burden_percent",
        modeled_fee_percent,
        metric_coverage(
            Partial,
            count,
            &[
                "entry_context.fees",
                "counterfactual_exit_fee_attribution.fee_schedule_id",
                "counterfactual_exit_fee_attribution.fee_schedule_version",
                "counterfactual_exit_fee_attribution.applied_rate",
            ],
            Some(
                "fee uses a sourced versioned schedule but remains modeled rather than independently settled candidate execution",
            ),
        ),
    );
    add_metric(
        &mut report,
        "time_normalized_return_percent_per_hour",
        time_normalized_return,
        metric_coverage(
            if time_normalized_return.is_some() {
                Partial
            } else {
                Unavailable
            },
            duration_values.len(),
            &["entry_timestamp_ms", "replayed_net_return_percent"],
            time_normalized_return
                .is_none()
                .then_some("entry time and replayed holding duration are required"),
        ),
    );
    add_metric(
        &mut report,
        "capital_efficiency_quote_pnl_per_quote_capital_day",
        capital_efficiency,
        metric_coverage(
            if capital_efficiency_covered == count && count > 0 {
                Partial
            } else {
                Unavailable
            },
            capital_efficiency_covered,
            &[
                "entry_context.initial_entry_notional",
                "entry_context.entry_fill_links[].fill_id",
                "entry_context.entry_fill_links[].timestamp_ms",
                "entry_context.entry_fill_links[].quantity",
                "entry_context.entry_fill_links[].execution_price",
                "candidate_replay.exit_timestamp_ms",
            ],
            (capital_efficiency_covered != count).then_some(
                "every entry fill needs a matching quote-currency notional and timestamp no later than the hypothetical replay exit; the candidate exit fee remains modeled",
            ),
        ),
    );
    add_metric(
        &mut report,
        "account_currency_expectancy_percent",
        None,
        metric_coverage(
            Unavailable,
            0,
            &["timestamped_fx_rate"],
            Some(
                "sourced timestamp-aligned FX conversion is not present in the exit replay contract",
            ),
        ),
    );

    // Direction is a supported context dimension. Market regime robustness
    // remains unavailable unless at least two explicitly labelled regimes
    // have adequate holdout coverage.
    let mut regimes = BTreeMap::<&str, Vec<(f64, f64)>>::new();
    let mut directions = BTreeMap::<&str, Vec<(f64, f64)>>::new();
    for row in paired {
        if let Some(regime) = row.0.market_regime.as_deref() {
            regimes.entry(regime).or_default().push((row.1, row.2));
        }
        directions
            .entry(if row.0.direction > 0.0 {
                "long"
            } else {
                "short"
            })
            .or_default()
            .push((row.1, row.2));
    }
    let robust_regimes = regimes.len() >= 2 && regimes.values().all(|rows| rows.len() >= 5);
    let worst_regime_improvement = if robust_regimes {
        let improvements = regimes
            .values()
            .map(|rows| rows.iter().map(|row| row.1 - row.0).sum::<f64>() / rows.len() as f64)
            .collect::<Vec<_>>();
        Some(improvements.iter().copied().fold(f64::INFINITY, f64::min))
    } else {
        None
    };
    add_metric(&mut report, "worst_market_regime_improvement_percent", worst_regime_improvement, metric_coverage(if robust_regimes { Complete } else { Unavailable }, regimes.values().map(Vec::len).sum(), &["market_regime", "baseline_replay_net_return_percent", "candidate_replay_net_return_percent"], (!robust_regimes).then_some("at least two explicitly labelled regimes with five holdout observations each are required")));
    let direction_robust = directions.len() == 2 && directions.values().all(|rows| rows.len() >= 5);
    let worst_direction_improvement = if direction_robust {
        directions
            .values()
            .map(|rows| rows.iter().map(|row| row.1 - row.0).sum::<f64>() / rows.len() as f64)
            .fold(f64::INFINITY, f64::min)
    } else {
        f64::NAN
    };
    add_metric(
        &mut report,
        "worst_direction_improvement_percent",
        direction_robust.then_some(worst_direction_improvement),
        metric_coverage(
            if direction_robust {
                Complete
            } else {
                Unavailable
            },
            directions.values().map(Vec::len).sum(),
            &[
                "side",
                "baseline_replay_net_return_percent",
                "candidate_replay_net_return_percent",
            ],
            (!direction_robust)
                .then_some("both long and short directions need five or more holdout observations"),
        ),
    );
    add_metric(&mut report, "measured_slippage_percent_of_notional", mean_measured_slippage_percent, metric_coverage(if measured_cost_covered == count && count > 0 { Complete } else if measured_cost_covered > 0 { Partial } else { Unavailable }, measured_cost_covered, &["measured_slippage_quote", "measured_slippage_currency", "entry_context.average_price", "entry_context.quantity"], (measured_cost_covered != count).then_some("measured slippage in the quote currency is missing or execution-cost evidence is incomplete for one or more replay outcomes")));

    add_metric(
        &mut report,
        "independently_reconciled_sample_coverage_percent",
        (count > 0).then(|| {
            paired.iter().filter(|row| row.0.independent_reconciliation_complete).count() as f64
                / count as f64
                * 100.0
        }),
        metric_coverage(
            if count > 0 && paired.iter().all(|row| row.0.independent_reconciliation_complete) {
                Complete
            } else if paired.iter().any(|row| row.0.independent_reconciliation_complete) {
                Partial
            } else {
                Unavailable
            },
            paired.iter().filter(|row| row.0.independent_reconciliation_complete).count(),
            &["independent_reconciliation.status", "independent_reconciliation.source", "independent_reconciliation.evidence_epoch_id"],
            (count == 0 || paired.iter().any(|row| !row.0.independent_reconciliation_complete)).then_some("an independent reconciled settlement source and evidence epoch are required for every candidate sample"),
        ),
    );

    if expectancy_r.is_none() {
        report
            .promotion_blockers
            .push("authoritative_initial_risk_unavailable".to_owned());
    }
    if measured_cost_covered != count || count == 0 {
        report
            .promotion_blockers
            .push("measured_execution_cost_coverage_incomplete".to_owned());
    }
    if !paired
        .iter()
        .all(|row| row.0.independent_reconciliation_complete)
        || count == 0
    {
        report
            .promotion_blockers
            .push("independent_settlement_reconciliation_unavailable".to_owned());
    }
    for metric in [
        "profit_factor",
        "payoff_ratio",
        "average_winner_percent",
        "average_loser_percent",
        "winner_preservation_rate",
        "loser_recovery_rate",
    ] {
        if !report.values.contains_key(metric) {
            report
                .promotion_blockers
                .push(format!("required_metric_unavailable:{metric}"));
        }
    }
    for (metric, blocker) in [
        (
            "mean_to_sample_standard_deviation",
            "risk_adjusted_return_unavailable",
        ),
        ("mean_position_duration_ms", "holding_duration_unavailable"),
    ] {
        if !report.values.contains_key(metric) {
            report.promotion_blockers.push(blocker.to_owned());
        }
    }
    if !report
        .values
        .contains_key("capital_efficiency_quote_pnl_per_quote_capital_day")
    {
        report
            .promotion_blockers
            .push("capital_efficiency_fill_timing_coverage_incomplete".to_owned());
    }
    if selected_hold_duration_days.is_some() && !duration_coverage_complete {
        report
            .promotion_blockers
            .push("maximum_hold_validation_coverage_incomplete".to_owned());
    }
    if !report
        .values
        .contains_key("account_currency_expectancy_percent")
    {
        report
            .promotion_blockers
            .push("sourced_timestamped_fx_unavailable".to_owned());
    }
    if !robust_regimes || worst_regime_improvement.is_some_and(|improvement| improvement < 0.0) {
        report
            .promotion_blockers
            .push("market_regime_robustness_unavailable".to_owned());
    }
    if !direction_robust || worst_direction_improvement < 0.0 {
        report
            .promotion_blockers
            .push("direction_robustness_unavailable".to_owned());
    }
    report.promotion_eligible = report.promotion_blockers.is_empty();
    report
}

fn counterfactual_exit_fee_quote(
    sample: &ReplayObservation,
    executable_price: f64,
    timestamp_ms: u64,
) -> Option<f64> {
    let quantity = sample.quantity;
    let attribution = &sample.counterfactual_exit_fee_attribution;
    if !executable_price.is_finite()
        || executable_price <= 0.0
        || !quantity.is_finite()
        || quantity <= 0.0
    {
        return None;
    }
    let rate = attribution.rate_by_timestamp.get(&timestamp_ms)?;
    let fee_currency = attribution.fee_currency_by_timestamp.get(&timestamp_ms)?;
    if fee_currency == &attribution.quote_currency {
        return (executable_price * quantity * rate)
            .is_finite()
            .then(|| executable_price * quantity * rate);
    }
    let rate_to_quote = attribution
        .fx_rate_to_quote_by_timestamp
        .get(&timestamp_ms)
        .copied()
        .flatten()
        .filter(|rate| rate.is_finite() && *rate > 0.0)?;
    let notional_in_fee_currency = executable_price * quantity / rate_to_quote;
    let fee_in_fee_currency = notional_in_fee_currency * rate;
    let fee_in_quote = fee_in_fee_currency * rate_to_quote;
    fee_in_quote.is_finite().then_some(fee_in_quote)
}

fn replay_outcome(
    sample: &ReplayObservation,
    parameters: ProfitProtectionParameters,
) -> Option<ReplayOutcome> {
    replay_outcome_with_hold_duration(sample, parameters, sample.maximum_holding_duration_days)
}

fn replay_outcome_with_hold_duration(
    sample: &ReplayObservation,
    parameters: ProfitProtectionParameters,
    maximum_holding_duration_days: Option<f64>,
) -> Option<ReplayOutcome> {
    let notional = sample.entry_price * sample.quantity;
    if !notional.is_finite() || notional <= 0.0 {
        return None;
    }
    let mut peak = f64::NEG_INFINITY;
    let mut exit = None;
    for quote in &sample.path {
        let executable_price = if sample.direction > 0.0 {
            quote.bid
        } else {
            quote.ask
        };
        let net_percent = replay_quote_return(sample, quote)?;
        peak = peak.max(net_percent);
        // Mirror live exit priority at sampled quote resolution: stop loss,
        // profit protection, then take profit.
        if net_percent <= -parameters.stop_loss_percent {
            exit = Some((
                executable_price,
                quote.timestamp_ms,
                ReplayExitReason::StopLoss,
            ));
            break;
        }
        if parameters
            .breakeven_activation_percent
            .is_some_and(|activation| peak >= activation && net_percent <= 0.0)
        {
            exit = Some((
                executable_price,
                quote.timestamp_ms,
                ReplayExitReason::BreakevenProtection,
            ));
            break;
        }
        if let (Some(activation), Some(distance)) = (
            parameters.trailing_activation_percent,
            parameters.trailing_distance_percent,
        ) && peak >= activation
            && net_percent <= peak - distance
        {
            exit = Some((
                executable_price,
                quote.timestamp_ms,
                ReplayExitReason::TrailingStop,
            ));
            break;
        }
        if peak >= parameters.activation_percent
            && net_percent
                <= (peak - parameters.giveback_percent).max(parameters.minimum_lock_percent)
        {
            exit = Some((
                executable_price,
                quote.timestamp_ms,
                ReplayExitReason::ProfitProtection,
            ));
            break;
        }
        if let (Some(entry_timestamp_ms), Some(hold_days)) =
            (sample.entry_timestamp_ms, maximum_holding_duration_days)
        {
            let maximum_hold_ms = hold_duration_days_to_ms(hold_days)?;
            if quote.timestamp_ms.saturating_sub(entry_timestamp_ms) >= maximum_hold_ms {
                exit = Some((
                    executable_price,
                    quote.timestamp_ms,
                    ReplayExitReason::MaximumHold,
                ));
                break;
            }
        }
        if net_percent >= parameters.take_profit_percent {
            exit = Some((
                executable_price,
                quote.timestamp_ms,
                ReplayExitReason::TakeProfit,
            ));
            break;
        }
    }
    let (price, exit_timestamp_ms, exit_reason) = match exit {
        Some(exit) => exit,
        None => {
            let quote = sample.path.last()?;
            (
                if sample.direction > 0.0 {
                    quote.bid
                } else {
                    quote.ask
                },
                quote.timestamp_ms,
                ReplayExitReason::Horizon,
            )
        }
    };
    let exit_fee_quote = counterfactual_exit_fee_quote(sample, price, exit_timestamp_ms)?;
    Some(ReplayOutcome {
        net_return_percent: ((price - sample.entry_price) * sample.quantity * sample.direction
            - sample.entry_fees
            - exit_fee_quote)
            / notional
            * 100.0,
        exit_price: price,
        exit_timestamp_ms,
        exit_reason,
    })
}

fn replay_quote_return(sample: &ReplayObservation, quote: &Quote) -> Option<f64> {
    let notional = sample.entry_price * sample.quantity;
    if !notional.is_finite() || notional <= 0.0 {
        return None;
    }
    let executable_price = if sample.direction > 0.0 {
        quote.bid
    } else {
        quote.ask
    };
    let exit_fee_quote =
        counterfactual_exit_fee_quote(sample, executable_price, quote.timestamp_ms)?;
    let net_percent =
        ((executable_price - sample.entry_price) * sample.quantity * sample.direction
            - sample.entry_fees
            - exit_fee_quote)
            / notional
            * 100.0;
    net_percent.is_finite().then_some(net_percent)
}

fn score_with_hold_duration(
    samples: &[ReplayObservation],
    parameters: ProfitProtectionParameters,
    maximum_holding_duration_days: Option<f64>,
) -> Option<f64> {
    let returns = samples
        .iter()
        .map(|sample| {
            replay_net_return_with_hold_duration(sample, parameters, maximum_holding_duration_days)
        })
        .collect::<Option<Vec<_>>>()?;
    (!returns.is_empty()).then(|| returns.iter().sum::<f64>() / returns.len() as f64)
}

fn baseline_score(samples: &[ReplayObservation]) -> Option<f64> {
    let returns = samples
        .iter()
        .map(|sample| replay_net_return(sample, sample.baseline))
        .collect::<Option<Vec<_>>>()?;
    (!returns.is_empty()).then(|| returns.iter().sum::<f64>() / returns.len() as f64)
}

fn mean_lower_bound_95(values: &[f64]) -> Option<f64> {
    if values.len() < 2 || values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64;
    Some(mean - 1.96 * (variance / values.len() as f64).sqrt())
}

fn nearest_rank_percentile(sorted_values: &[f64], quantile: f64) -> Option<f64> {
    if sorted_values.is_empty()
        || !quantile.is_finite()
        || !(0.0..=1.0).contains(&quantile)
        || sorted_values.iter().any(|value| !value.is_finite())
    {
        return None;
    }
    let rank = ((sorted_values.len() as f64 * quantile).ceil() as usize)
        .saturating_sub(1)
        .min(sorted_values.len() - 1);
    sorted_values.get(rank).copied()
}

fn passes_downside_gate(
    negative_difference_rate: f64,
    tenth_percentile_difference: f64,
    worst_difference: f64,
) -> bool {
    negative_difference_rate.is_finite()
        && tenth_percentile_difference.is_finite()
        && worst_difference.is_finite()
        && negative_difference_rate <= MAX_VALIDATION_NEGATIVE_DIFFERENCE_RATE
        && tenth_percentile_difference >= MIN_VALIDATION_TENTH_PERCENTILE_DIFFERENCE_PERCENT
        && worst_difference >= MIN_VALIDATION_WORST_DIFFERENCE_PERCENT
}

#[cfg(test)]
fn candidate_values(min: f64, max: f64, step: f64) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() || !step.is_finite() || step <= 0.0 || max < min {
        return Vec::new();
    }
    let count = (((max - min) / step).floor() as usize + 1).min(MAX_CANDIDATES_PER_PARAMETER);
    (0..count)
        .map(|index| (min + index as f64 * step).min(max))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote_currency_fee_attribution(
        rate: f64,
        timestamps: &[u64],
    ) -> CounterfactualExitFeeAttribution {
        CounterfactualExitFeeAttribution {
            rate_by_timestamp: timestamps
                .iter()
                .map(|timestamp| (*timestamp, rate))
                .collect(),
            fee_currency_by_timestamp: timestamps
                .iter()
                .map(|timestamp| (*timestamp, "USDT".to_owned()))
                .collect(),
            quote_currency: "USDT".into(),
            fx_rate_to_quote_by_timestamp: timestamps
                .iter()
                .map(|timestamp| (*timestamp, None))
                .collect(),
        }
    }

    #[derive(Debug)]
    struct ReplayReport {
        baseline_mean_net_percent: f64,
        selected_mean_net_percent: f64,
        baseline_max_drawdown_percent: f64,
        selected_max_drawdown_percent: f64,
        actual_exit_reason_counts: [usize; 3],
        baseline_replay_exit_reason_counts: [usize; 7],
        selected_replay_exit_reason_counts: [usize; 7],
        hold_improvement_mean_percent: f64,
        hold_improvement_median_percent: f64,
        hold_improvement_positive_rate: f64,
        long_holdout: DirectionSummary,
        short_holdout: DirectionSummary,
    }

    #[derive(Debug)]
    struct DirectionSummary {
        sample_count: usize,
        baseline_mean_net_percent: f64,
        selected_mean_net_percent: f64,
        hold_improvement_mean_percent: f64,
        hold_improvement_median_percent: f64,
        hold_improvement_positive_rate: f64,
        actual_exit_reason_counts: [usize; 3],
    }

    fn report_replay(
        samples: &[ReplayObservation],
        selected: ProfitProtectionParameters,
        selected_hold_duration_days: Option<f64>,
    ) -> ReplayReport {
        let baseline = samples
            .iter()
            .map(|sample| replay_outcome(sample, sample.baseline).unwrap())
            .collect::<Vec<_>>();
        let candidate = samples
            .iter()
            .map(|sample| {
                replay_outcome_with_hold_duration(sample, selected, selected_hold_duration_days)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let hold_improvements = samples
            .iter()
            .map(|sample| {
                let hold = replay_outcome(
                    sample,
                    ProfitProtectionParameters {
                        stop_loss_percent: 0.5,
                        take_profit_percent: 0.8,
                        activation_percent: f64::MAX,
                        giveback_percent: 0.3,
                        minimum_lock_percent: 0.0,
                        trailing_activation_percent: None,
                        trailing_distance_percent: None,
                        breakeven_activation_percent: None,
                    },
                )
                .unwrap();
                hold.net_return_percent - sample.actual_exit_net_return_percent
            })
            .collect::<Vec<_>>();
        let mut sorted_hold_improvements = hold_improvements.clone();
        sorted_hold_improvements.sort_by(f64::total_cmp);
        ReplayReport {
            baseline_mean_net_percent: mean(
                &baseline
                    .iter()
                    .map(|outcome| outcome.net_return_percent)
                    .collect::<Vec<_>>(),
            ),
            selected_mean_net_percent: mean(
                &candidate
                    .iter()
                    .map(|outcome| outcome.net_return_percent)
                    .collect::<Vec<_>>(),
            ),
            baseline_max_drawdown_percent: max_drawdown(
                &baseline
                    .iter()
                    .map(|outcome| outcome.net_return_percent)
                    .collect::<Vec<_>>(),
            ),
            selected_max_drawdown_percent: max_drawdown(
                &candidate
                    .iter()
                    .map(|outcome| outcome.net_return_percent)
                    .collect::<Vec<_>>(),
            ),
            actual_exit_reason_counts: [
                samples
                    .iter()
                    .filter(|sample| sample.exit_reason == "profit-protection")
                    .count(),
                samples
                    .iter()
                    .filter(|sample| sample.exit_reason == "take-profit")
                    .count(),
                samples
                    .iter()
                    .filter(|sample| sample.exit_reason == "stop-loss")
                    .count(),
            ],
            baseline_replay_exit_reason_counts: reason_counts(&baseline),
            selected_replay_exit_reason_counts: reason_counts(&candidate),
            hold_improvement_mean_percent: mean(&hold_improvements),
            hold_improvement_median_percent: sorted_hold_improvements
                [sorted_hold_improvements.len() / 2],
            hold_improvement_positive_rate: hold_improvements
                .iter()
                .filter(|value| **value > 0.0)
                .count() as f64
                / hold_improvements.len() as f64,
            long_holdout: direction_summary(
                &samples
                    .iter()
                    .filter(|sample| sample.direction > 0.0)
                    .cloned()
                    .collect::<Vec<_>>(),
                selected,
                selected_hold_duration_days,
            ),
            short_holdout: direction_summary(
                &samples
                    .iter()
                    .filter(|sample| sample.direction < 0.0)
                    .cloned()
                    .collect::<Vec<_>>(),
                selected,
                selected_hold_duration_days,
            ),
        }
    }

    fn direction_summary(
        samples: &[ReplayObservation],
        selected: ProfitProtectionParameters,
        selected_hold_duration_days: Option<f64>,
    ) -> DirectionSummary {
        let baseline = samples
            .iter()
            .map(|sample| {
                replay_outcome(sample, sample.baseline)
                    .unwrap()
                    .net_return_percent
            })
            .collect::<Vec<_>>();
        let candidate = samples
            .iter()
            .map(|sample| {
                replay_outcome_with_hold_duration(sample, selected, selected_hold_duration_days)
                    .unwrap()
                    .net_return_percent
            })
            .collect::<Vec<_>>();
        let mut hold_improvements = samples
            .iter()
            .map(|sample| {
                replay_outcome(
                    sample,
                    ProfitProtectionParameters {
                        stop_loss_percent: 0.5,
                        take_profit_percent: 0.8,
                        activation_percent: f64::MAX,
                        giveback_percent: 0.3,
                        minimum_lock_percent: 0.0,
                        trailing_activation_percent: None,
                        trailing_distance_percent: None,
                        breakeven_activation_percent: None,
                    },
                )
                .unwrap()
                .net_return_percent
                    - sample.actual_exit_net_return_percent
            })
            .collect::<Vec<_>>();
        hold_improvements.sort_by(f64::total_cmp);
        DirectionSummary {
            sample_count: samples.len(),
            baseline_mean_net_percent: mean(&baseline),
            selected_mean_net_percent: mean(&candidate),
            hold_improvement_mean_percent: mean(&hold_improvements),
            hold_improvement_median_percent: hold_improvements[hold_improvements.len() / 2],
            hold_improvement_positive_rate: hold_improvements
                .iter()
                .filter(|value| **value > 0.0)
                .count() as f64
                / hold_improvements.len() as f64,
            actual_exit_reason_counts: [
                samples
                    .iter()
                    .filter(|sample| sample.exit_reason == "profit-protection")
                    .count(),
                samples
                    .iter()
                    .filter(|sample| sample.exit_reason == "take-profit")
                    .count(),
                samples
                    .iter()
                    .filter(|sample| sample.exit_reason == "stop-loss")
                    .count(),
            ],
        }
    }

    fn mean(values: &[f64]) -> f64 {
        values.iter().sum::<f64>() / values.len() as f64
    }

    #[test]
    fn capital_efficiency_uses_candidate_exit_and_time_weighted_fill_notionals() {
        let day_ms = MS_PER_DAY as u64;
        let sample = ReplayObservation {
            event_id: "capital-efficiency-fixture".into(),
            exit_timestamp_ms: day_ms + 1,
            observed_at_ms: day_ms + 1,
            direction: 1.0,
            entry_price: 100.0,
            quantity: 2.0,
            entry_fees: 0.1,
            initial_risk_quote: None,
            initial_risk_currency: None,
            quote_currency: Some("USDT".into()),
            market_regime: None,
            entry_timestamp_ms: Some(1),
            maximum_holding_duration_days: None,
            profile_id: Some("profile-v1".into()),
            profile_version: Some(1),
            initial_entry_notional_quote: Some(200.0),
            entry_fill_timing: Some(vec![
                EntryFillTiming {
                    timestamp_ms: 1,
                    notional_quote: 100.0,
                },
                EntryFillTiming {
                    timestamp_ms: day_ms / 2 + 1,
                    notional_quote: 100.0,
                },
            ]),
            execution_costs_complete: false,
            counterfactual_exit_fee_attribution: quote_currency_fee_attribution(0.001, &[]),
            gross_executable_mfe_percent: 0.0,
            gross_executable_mae_percent: 0.0,
            net_executable_mfe_percent: 0.0,
            net_executable_mae_percent: 0.0,
            measured_slippage_quote: None,
            measured_slippage_currency: None,
            independent_reconciliation_complete: false,
            stop_loss_percent: 0.5,
            take_profit_percent: 0.8,
            baseline: ProfitProtectionParameters {
                stop_loss_percent: 0.5,
                take_profit_percent: 0.8,
                activation_percent: 0.2,
                giveback_percent: 0.15,
                minimum_lock_percent: 0.1,
                trailing_activation_percent: None,
                trailing_distance_percent: None,
                breakeven_activation_percent: None,
            },
            actual_exit_net_return_percent: 3.0,
            exit_reason: "take-profit".into(),
            path: Vec::new(),
        };
        let hypothetical = ReplayOutcome {
            net_return_percent: 3.0,
            exit_price: 103.0,
            exit_timestamp_ms: day_ms + 1,
            exit_reason: ReplayExitReason::TakeProfit,
        };
        assert_eq!(
            capital_efficiency_sample(&sample, hypothetical),
            Some((6.0, 150.0))
        );

        let mut post_exit_fill = sample.clone();
        post_exit_fill.entry_fill_timing.as_mut().unwrap()[1].timestamp_ms = day_ms + 2;
        assert_eq!(
            capital_efficiency_sample(&post_exit_fill, hypothetical),
            None
        );
        let mut mismatched_notional = sample;
        mismatched_notional.initial_entry_notional_quote = Some(201.0);
        assert_eq!(
            capital_efficiency_sample(&mismatched_notional, hypothetical),
            None
        );
    }

    fn reason_counts(outcomes: &[ReplayOutcome]) -> [usize; 7] {
        let mut counts = [0; 7];
        for outcome in outcomes {
            counts[match outcome.exit_reason {
                ReplayExitReason::StopLoss => 0,
                ReplayExitReason::BreakevenProtection => 1,
                ReplayExitReason::TrailingStop => 2,
                ReplayExitReason::ProfitProtection => 3,
                ReplayExitReason::MaximumHold => 4,
                ReplayExitReason::TakeProfit => 5,
                ReplayExitReason::Horizon => 6,
            }] += 1;
        }
        counts
    }

    #[derive(Clone, Copy)]
    enum PathCase {
        TrainingAndHoldoutImprove,
        HoldoutReverses,
        HoldoutFlat,
    }

    fn synthetic_observations(case: PathCase) -> Vec<Value> {
        (0..40)
            .map(|index| {
                // Leave room for the authoritative entry timestamp preceding
                // each synthetic exit; unsigned subtraction must not wrap.
                let timestamp = 10_000_000 + index as u64 * 10_000_000;
                let holdout = index >= 30;
                let is_long = index % 2 == 0;
                let exit_kind = index % 5;
                let mut executable_prices = match exit_kind {
                    0..=2 => match (holdout, case) {
                        (true, PathCase::HoldoutReverses) => {
                            vec![100.10, 100.35, 100.50, 100.25, 100.10, 99.90, 99.80]
                        }
                        (true, PathCase::HoldoutFlat) => {
                            vec![100.10, 100.35, 100.50, 100.25, 100.25, 100.25, 100.25]
                        }
                        _ => vec![100.10, 100.35, 100.50, 100.25, 100.40, 100.40, 100.40],
                    },
                    3 => match (holdout, case) {
                        (true, PathCase::TrainingAndHoldoutImprove) => {
                            vec![100.10, 100.35, 100.35, 100.35, 100.10, 100.10, 100.10]
                        }
                        _ => vec![100.10, 100.50, 101.00, 101.00, 101.00, 101.00, 101.00],
                    },
                    _ => vec![99.90, 99.50, 99.30, 99.30, 99.30, 99.30, 99.30],
                };
                if !is_long {
                    executable_prices = executable_prices
                        .into_iter()
                        .map(|price| 200.0 - price)
                        .collect();
                }
                let mut path = Vec::new();
                for minute in 0..=30 {
                    let price_index = minute.min(executable_prices.len() - 1);
                    let price = executable_prices[price_index];
                    path.push(serde_json::json!({
                        "timestamp": timestamp + minute as u64 * 60_000,
                        "bid": if is_long { price } else { price - 0.02 },
                        "ask": if is_long { price + 0.02 } else { price }
                    }));
                }
                let (reason, actual_return) = match exit_kind {
                    0..=2 => ("profit-protection", 0.14),
                    3 if holdout && matches!(case, PathCase::TrainingAndHoldoutImprove) => {
                        ("profit-protection", 0.08)
                    }
                    3 => ("take-profit", 0.89),
                    _ => ("stop-loss", -0.81),
                };
                let mut observation = serde_json::json!({
                    "event_id": format!("synthetic-{index}"),
                    "symbol": "BTC-USDT",
                    "strategy": "live-momentum",
                    "reason": reason,
                    "side": if is_long { "sell" } else { "buy" },
                    "exit_timestamp_ms": timestamp,
                    "observed_at_ms": timestamp + 30 * 60_000,
                    "horizon_ms": 30 * 60_000,
                    "entry_context": {
                        "average_price": 100.0,
                        "quantity": 1.0,
                        "fees": 0.01,
                        "timestamp_ms": timestamp - 3_600_000,
                        "entry_fill_costs_complete": true,
                        "entry_fill_ids": ["entry-fill-1"],
                        "entry_fee_currencies": ["USDT"],
                        "entry_fill_links": [{"fill_id":"entry-fill-1", "fee_known":true, "fee_amount":0.01, "fee_currency":"USDT", "timestamp_ms":timestamp - 3_600_000, "quantity":1.0, "execution_price":100.0}],
                        "initial_entry_notional": 100.0,
                        "initial_risk_reference": {"schema_version":1, "status":"complete", "quote_amount":0.5, "quote_currency":"USDT", "fill_ids":["entry-fill-1"]},
                        "strategy_decision": {"learning_profile_id":"synthetic-profile-v1", "learning_profile_version":1}
                    },
                    // Deterministic test-only evidence for fields captured by
                    // the current contracts; missing FX/cost/reconciliation
                    // inputs below remain explicitly unavailable.
                    "exit_fill_evidence": {"fees_complete":true},
                    "realized_r_evidence": {"schema_version":1, "status":"complete", "fee_coverage_complete":true, "initial_risk_quote_amount":0.5, "quote_currency":"USDT", "expected_quote_currency":"USDT", "initial_risk_fill_ids":["entry-fill-1"]},
                    "pre_exit_price_path_evidence": {"complete":true, "identity_complete":true},
                    "price_path_evidence": {"complete":true},
                    "independent_reconciliation": {"status":"reconciled", "source":"paper_exchange_execution_ledger", "evidence_epoch_id":"fixture-paper-epoch"},
                    "exit_parameter_provenance": {"profile_id":"synthetic-profile-v1", "profile_version":1},
                    "market_regime": if index % 10 < 5 { "trend" } else { "range" },
                    "exit_return_percent": actual_return,
                    "counterfactual_exit_fee_rate": 0.001,
                    "effective_stop_loss_percent": 0.5,
                    "effective_take_profit_percent": 0.8,
                    "strategy_decision": {"strategy_parameters": {
                        "profit_protection_activation_percent": 0.15,
                        "profit_protection_giveback_percent": 0.10,
                        "profit_protection_minimum_lock_percent": 0.05
                    }},
                    "pre_exit_price_path": [{"timestamp": timestamp - 1, "bid": 100.0, "ask": 100.02}],
                    "price_path": path
                });
                add_synthetic_fee_schedule_attribution(&mut observation);
                observation
            })
            .collect()
    }

    fn add_synthetic_fee_schedule_attribution(observation: &mut Value) {
        let event_id = observation["event_id"].as_str().unwrap().to_owned();
        let symbol = observation["symbol"].as_str().unwrap().to_owned();
        let schedule_id = "fixture-fee-schedule";
        let schedule_version = "v1";
        let epoch = "fixture-feed-epoch";
        let mut quote_observations = Vec::new();
        let mut sequence = 0_u64;
        for key in ["pre_exit_price_path", "price_path"] {
            for quote in observation[key].as_array_mut().unwrap() {
                sequence += 1;
                let timestamp = quote["timestamp"].as_u64().unwrap();
                let updated_at = timestamp.saturating_sub(1);
                quote["epoch"] = serde_json::json!(epoch);
                quote["sequence"] = serde_json::json!(sequence);
                quote["source_updated_at_ms"] = serde_json::json!(updated_at);
                quote["freshness_contract_version"] = serde_json::json!(1);
                quote["freshness_max_age_ms"] = serde_json::json!(5_000);
                quote["source_fresh"] = serde_json::json!(true);
                quote["quote_source_system"] = serde_json::json!("feed_handlers");
                quote["quote_source_epoch"] = serde_json::json!(epoch);
                quote["quote_source_cursor"] = serde_json::json!(sequence);
                quote["quote_source_record_id"] =
                    serde_json::json!(format!("fixture-quote-{event_id}-{sequence}"));
                quote_observations.push(serde_json::json!({
                    "timestamp_ms":timestamp,
                    "quote_epoch":epoch,
                    "quote_sequence":sequence,
                    "quote_source_updated_at_ms":updated_at,
                    "quote_freshness_contract_version":1,
                    "quote_freshness_max_age_ms":5_000,
                    "quote_source_fresh":true,
                    "quote_source_system":"feed_handlers",
                    "quote_source_epoch":epoch,
                    "quote_source_cursor":sequence,
                    "quote_source_record_id":format!("fixture-quote-{event_id}-{sequence}"),
                    "fee_schedule_id":schedule_id,
                    "fee_schedule_version":schedule_version,
                    "exit_order_class":"taker",
                    "applied_rate":0.001,
                    "fee_currency":"USDT",
                    "fee_currency_conversion":null
                }));
            }
        }
        let first_timestamp = observation["entry_context"]["timestamp_ms"]
            .as_u64()
            .unwrap();
        let exit_timestamp = observation["exit_timestamp_ms"].as_u64().unwrap();
        let fee_schedule_event = serde_json::json!({
            "schema_version":"paper_exchange_financial_evidence:v1",
            "event_type":"fee_schedule",
            "event_id":"fixture-fee-schedule-event-17",
            "source_system":"paper_exchange_simulator",
            "source_epoch":"fixture-paper-ledger-epoch",
            "source_cursor":17,
            "source_record_id":"fixture-fee-schedule-record-17",
            "source_timestamp_utc_ms":first_timestamp,
            "fee_schedule_id":schedule_id,
            "fee_schedule_version":schedule_version,
            "symbol":symbol,
            "effective_from_utc_ms":first_timestamp,
            "effective_to_utc_ms":null,
            "maker_rate":0.0008,
            "taker_rate":0.001
        });
        observation["entry_context"]["entry_fill_links"][0]["paper_exchange_fee_amount"] =
            serde_json::json!(0.01);
        observation["entry_context"]["entry_fill_links"][0]["paper_exchange_fee_currency"] =
            serde_json::json!("USDT");
        observation["entry_context"]["entry_fill_links"][0]["paper_exchange_fee_schedule_id"] =
            serde_json::json!(schedule_id);
        observation["entry_context"]["entry_fill_links"][0]["paper_exchange_fee_schedule_version"] =
            serde_json::json!(schedule_version);
        observation["entry_context"]["entry_fill_links"][0]["paper_exchange_fee_order_class"] =
            serde_json::json!("taker");
        observation["exit_fill_evidence"]["fills"] = serde_json::json!([{
            "fee_amount":0.01,
            "fee_currency":"USDT",
            "paper_exchange_fee_amount":0.01,
            "paper_exchange_fee_currency":"USDT",
            "paper_exchange_fee_schedule_id":schedule_id,
            "paper_exchange_fee_schedule_version":schedule_version,
            "paper_exchange_fee_order_class":"taker"
        }]);
        observation["counterfactual_exit_fee_attribution"] = serde_json::json!({
            "schema_version":1,
            "fee_schedules":[{
                "source_schema_version":fee_schedule_event["schema_version"],
                "source_event_id":fee_schedule_event["event_id"],
                "source_system":fee_schedule_event["source_system"],
                "source_epoch":fee_schedule_event["source_epoch"],
                "source_cursor":fee_schedule_event["source_cursor"],
                "source_record_id":fee_schedule_event["source_record_id"],
                "source_timestamp_utc_ms":fee_schedule_event["source_timestamp_utc_ms"],
                "fee_schedule_id":fee_schedule_event["fee_schedule_id"],
                "fee_schedule_version":fee_schedule_event["fee_schedule_version"],
                "symbol":fee_schedule_event["symbol"],
                "effective_from_utc_ms":fee_schedule_event["effective_from_utc_ms"],
                "effective_to_utc_ms":fee_schedule_event["effective_to_utc_ms"],
                "maker_rate":fee_schedule_event["maker_rate"],
                "taker_rate":fee_schedule_event["taker_rate"]
            }],
            "exit_fee_schedule_ref":{
                "timestamp_ms":exit_timestamp,
                "fee_schedule_id":schedule_id,
                "fee_schedule_version":schedule_version,
                "exit_order_class":"taker",
                "applied_rate":0.001,
                "fee_currency":"USDT",
                "fee_currency_conversion":null
            },
            "quote_observations":quote_observations
        });
    }

    #[test]
    fn versioned_fee_schedule_produces_gross_and_net_executable_excursions() {
        let mut observation = synthetic_observations(PathCase::TrainingAndHoldoutImprove).remove(0);
        let exit_timestamp = observation["exit_timestamp_ms"].as_u64().unwrap();
        observation["pre_exit_price_path"] = serde_json::json!([
            {"timestamp":exit_timestamp - 30_000,"bid":100.10,"ask":100.12},
            {"timestamp":exit_timestamp - 20_000,"bid":99.50,"ask":99.52},
            {"timestamp":exit_timestamp - 10_000,"bid":100.60,"ask":100.62}
        ]);
        add_synthetic_fee_schedule_attribution(&mut observation);
        observation
            .as_object_mut()
            .unwrap()
            .remove("counterfactual_exit_fee_rate");

        let sample = parse_observation(&observation, "BTC-USDT", "live-momentum", u64::MAX)
            .expect("complete versioned fee attribution permits net replay");
        assert!((sample.gross_executable_mfe_percent - 0.60).abs() < 1e-9);
        assert!((sample.gross_executable_mae_percent + 0.50).abs() < 1e-9);
        assert!((sample.net_executable_mfe_percent - 0.4894).abs() < 1e-8);
        assert!((sample.net_executable_mae_percent + 0.6095).abs() < 1e-8);
        assert!(
            sample
                .counterfactual_exit_fee_attribution
                .rate_by_timestamp
                .values()
                .all(|rate| *rate == 0.001)
        );
        assert!(
            sample
                .counterfactual_exit_fee_attribution
                .fee_currency_by_timestamp
                .values()
                .all(|currency| currency == "USDT")
        );
        assert!(
            sample
                .counterfactual_exit_fee_attribution
                .fx_rate_to_quote_by_timestamp
                .values()
                .all(Option::is_none)
        );

        let excursion_quote = sample
            .path
            .iter()
            .find(|quote| quote.bid == 100.60)
            .unwrap();
        assert!((replay_quote_return(&sample, excursion_quote).unwrap() - 0.4894).abs() < 1e-8);
    }

    #[test]
    fn versioned_currency_conversion_is_required_and_applied_at_each_quote_time() {
        let mut observation = synthetic_observations(PathCase::TrainingAndHoldoutImprove).remove(0);
        let exit_timestamp = observation["exit_timestamp_ms"].as_u64().unwrap();
        observation["pre_exit_price_path"] = serde_json::json!([
            {"timestamp":exit_timestamp - 10_000,"bid":100.10,"ask":100.12},
            {"timestamp":exit_timestamp - 5_000,"bid":100.60,"ask":100.62}
        ]);
        add_synthetic_fee_schedule_attribution(&mut observation);
        let quote_observations =
            observation["counterfactual_exit_fee_attribution"]["quote_observations"]
                .as_array_mut()
                .unwrap();
        for quote in quote_observations {
            let timestamp = quote["timestamp_ms"].as_u64().unwrap();
            quote["fee_currency"] = serde_json::json!("BNB");
            quote["fee_currency_conversion"] = serde_json::json!({
                "schema_version":1,
                "source_schema_version":"paper_exchange_financial_evidence:v1",
                "from_currency":"BNB",
                "to_currency":"USDT",
                "rate_to_quote":600.0,
                "effective_from_utc_ms":timestamp - 10_000,
                "effective_to_utc_ms":null,
                "source_system":"paper_exchange_simulator_account",
                "source_epoch":"fx-epoch-v1",
                "source_cursor":timestamp,
                "source_record_id":format!("fx-mark-{timestamp}"),
                "source_timestamp_utc_ms":timestamp
            });
        }
        observation["counterfactual_exit_fee_attribution"]["exit_fee_schedule_ref"]["fee_currency"] =
            serde_json::json!("BNB");
        observation["counterfactual_exit_fee_attribution"]["exit_fee_schedule_ref"]["fee_currency_conversion"] = serde_json::json!({
            "schema_version":1,
            "source_schema_version":"paper_exchange_financial_evidence:v1",
            "from_currency":"BNB",
            "to_currency":"USDT",
            "rate_to_quote":600.0,
            "effective_from_utc_ms":exit_timestamp - 10_000,
            "effective_to_utc_ms":null,
            "source_system":"paper_exchange_simulator_account",
            "source_epoch":"fx-epoch-v1",
            "source_cursor":exit_timestamp,
            "source_record_id":format!("fx-mark-{exit_timestamp}"),
            "source_timestamp_utc_ms":exit_timestamp
        });
        let sample = parse_observation(&observation, "BTC-USDT", "live-momentum", u64::MAX)
            .expect("timestamped sourced conversion permits fee normalization");
        assert!((sample.net_executable_mfe_percent - 0.4894).abs() < 1e-8);
        assert!(
            sample
                .counterfactual_exit_fee_attribution
                .fx_rate_to_quote_by_timestamp
                .values()
                .all(|rate| *rate == Some(600.0))
        );
    }

    #[test]
    fn replay_attributes_each_quote_to_one_effective_fee_schedule_version() {
        let mut observation = synthetic_observations(PathCase::TrainingAndHoldoutImprove).remove(0);
        let exit_timestamp = observation["exit_timestamp_ms"].as_u64().unwrap();
        observation["pre_exit_price_path"] = serde_json::json!([
            {"timestamp":exit_timestamp - 20_000,"bid":100.10,"ask":100.12},
            {"timestamp":exit_timestamp - 10_000,"bid":100.60,"ask":100.62}
        ]);
        add_synthetic_fee_schedule_attribution(&mut observation);
        let first_schedule =
            observation["counterfactual_exit_fee_attribution"]["fee_schedules"][0].clone();
        let mut first_schedule = first_schedule;
        first_schedule["effective_to_utc_ms"] = serde_json::json!(exit_timestamp);
        let mut second_schedule = first_schedule.clone();
        second_schedule["fee_schedule_id"] = serde_json::json!("fixture-fee-schedule-next");
        second_schedule["fee_schedule_version"] = serde_json::json!("v2");
        second_schedule["source_cursor"] = serde_json::json!(18);
        second_schedule["source_record_id"] = serde_json::json!("fixture-fee-schedule-record-18");
        second_schedule["source_timestamp_utc_ms"] = serde_json::json!(exit_timestamp);
        second_schedule["effective_from_utc_ms"] = serde_json::json!(exit_timestamp);
        second_schedule["effective_to_utc_ms"] = serde_json::Value::Null;
        second_schedule["taker_rate"] = serde_json::json!(0.0015);
        observation["counterfactual_exit_fee_attribution"]["fee_schedules"] =
            serde_json::json!([first_schedule, second_schedule]);
        for quote in observation["counterfactual_exit_fee_attribution"]["quote_observations"]
            .as_array_mut()
            .unwrap()
        {
            if quote["timestamp_ms"].as_u64().unwrap() >= exit_timestamp {
                quote["fee_schedule_id"] = serde_json::json!("fixture-fee-schedule-next");
                quote["fee_schedule_version"] = serde_json::json!("v2");
                quote["applied_rate"] = serde_json::json!(0.0015);
            }
        }
        observation["counterfactual_exit_fee_attribution"]["exit_fee_schedule_ref"] = serde_json::json!({
            "timestamp_ms":exit_timestamp,
            "fee_schedule_id":"fixture-fee-schedule-next",
            "fee_schedule_version":"v2",
            "exit_order_class":"taker",
            "applied_rate":0.0015,
            "fee_currency":"USDT",
            "fee_currency_conversion":null
        });

        let sample = parse_observation(&observation, "BTC-USDT", "live-momentum", u64::MAX)
            .expect("each path quote and close timestamp resolve to one versioned schedule");
        assert_eq!(
            sample.counterfactual_exit_fee_attribution.rate_by_timestamp
                [&(exit_timestamp - 10_000)],
            0.001
        );
        assert_eq!(
            sample.counterfactual_exit_fee_attribution.rate_by_timestamp[&exit_timestamp],
            0.0015
        );

        let mut gap = observation.clone();
        gap["counterfactual_exit_fee_attribution"]["fee_schedules"][1]["effective_from_utc_ms"] =
            serde_json::json!(exit_timestamp + 1);
        assert!(parse_observation(&gap, "BTC-USDT", "live-momentum", u64::MAX).is_none());

        let mut overlap = observation;
        overlap["counterfactual_exit_fee_attribution"]["fee_schedules"][0]["effective_to_utc_ms"] =
            serde_json::json!(exit_timestamp + 1);
        assert!(parse_observation(&overlap, "BTC-USDT", "live-momentum", u64::MAX).is_none());
    }

    #[test]
    fn missing_mismatched_expired_or_unconverted_fees_keep_gross_only_and_block_replay() {
        let mut base = synthetic_observations(PathCase::TrainingAndHoldoutImprove).remove(0);
        let exit_timestamp = base["exit_timestamp_ms"].as_u64().unwrap();
        base["pre_exit_price_path"] = serde_json::json!([
            {"timestamp":exit_timestamp - 10_000,"bid":100.10,"ask":100.12},
            {"timestamp":exit_timestamp - 5_000,"bid":100.60,"ask":100.62}
        ]);
        add_synthetic_fee_schedule_attribution(&mut base);

        let mut absent = base.clone();
        absent
            .as_object_mut()
            .unwrap()
            .remove("counterfactual_exit_fee_attribution");
        assert_eq!(absent["counterfactual_exit_fee_rate"], 0.001);

        let mut mismatched = base.clone();
        mismatched["counterfactual_exit_fee_attribution"]["quote_observations"][0]["fee_schedule_version"] =
            serde_json::json!("v2");

        let mut expired = base.clone();
        expired["counterfactual_exit_fee_attribution"]["fee_schedules"][0]["effective_to_utc_ms"] =
            serde_json::json!(exit_timestamp);

        let mut currency_unconverted = base.clone();
        for quote in
            currency_unconverted["counterfactual_exit_fee_attribution"]["quote_observations"]
                .as_array_mut()
                .unwrap()
        {
            quote["fee_currency"] = serde_json::json!("BNB");
        }
        let absent_assessment_input = absent.clone();

        for invalid in [absent, mismatched, expired, currency_unconverted] {
            assert!(
                parse_observation(&invalid, "BTC-USDT", "live-momentum", u64::MAX).is_none(),
                "unsupported fee attribution cannot enter net replay: {invalid}"
            );
            let gross_path = invalid["pre_exit_price_path"]
                .as_array()
                .unwrap()
                .iter()
                .map(parse_quote)
                .collect::<Option<Vec<_>>>()
                .unwrap();
            let (mfe, mae) = gross_executable_excursion_extrema(100.0, 1.0, 1.0, &gross_path)
                .expect("gross excursion remains available independently");
            assert!((mfe - 0.60).abs() < 1e-9);
            assert!((mae - 0.10).abs() < 1e-9);
        }
        let gross_only_assessment = unavailable_exit_policy_assessment(
            std::iter::once(&absent_assessment_input),
            "BTC-USDT",
            "live-momentum",
            u64::MAX,
        );
        assert!(
            (gross_only_assessment.metrics.values["gross_executable_mfe_percent"] - 0.60).abs()
                < 1e-9
        );
        assert!(
            (gross_only_assessment.metrics.values["gross_executable_mae_percent"] - 0.10).abs()
                < 1e-9
        );
        assert!(
            !gross_only_assessment
                .metrics
                .values
                .contains_key("net_executable_mfe_percent")
        );
        assert_eq!(
            gross_only_assessment.metrics.coverage
                ["counterfactual_exit_fee_schedule_source_coverage_percent"]
                .sample_count,
            0
        );
    }

    fn parsed_synthetic(case: PathCase) -> Vec<ReplayObservation> {
        synthetic_observations(case)
            .iter()
            .filter_map(|value| parse_observation(value, "BTC-USDT", "live-momentum", u64::MAX))
            .map(|mut sample| {
                // Keep actual exit outcomes aligned with the captured baseline
                // replay, while retaining the intentionally mixed exit reasons.
                sample.actual_exit_net_return_percent =
                    replay_net_return(&sample, sample.baseline).unwrap();
                sample
            })
            .collect()
    }

    #[test]
    fn uses_executable_side_and_charges_both_fees() {
        let sample = ReplayObservation {
            event_id: "long-1".into(),
            exit_timestamp_ms: 1,
            observed_at_ms: 3,
            direction: 1.0,
            entry_price: 100.0,
            quantity: 1.0,
            entry_fees: 0.1,
            initial_risk_quote: None,
            initial_risk_currency: None,
            quote_currency: None,
            market_regime: None,
            entry_timestamp_ms: None,
            maximum_holding_duration_days: None,
            profile_id: None,
            profile_version: None,
            initial_entry_notional_quote: None,
            entry_fill_timing: None,
            execution_costs_complete: false,
            counterfactual_exit_fee_attribution: quote_currency_fee_attribution(0.001, &[2, 3]),
            gross_executable_mfe_percent: 0.0,
            gross_executable_mae_percent: 0.0,
            net_executable_mfe_percent: 0.0,
            net_executable_mae_percent: 0.0,
            measured_slippage_quote: None,
            measured_slippage_currency: None,
            independent_reconciliation_complete: false,
            stop_loss_percent: 5.0,
            take_profit_percent: 5.0,
            baseline: ProfitProtectionParameters {
                stop_loss_percent: 0.5,
                take_profit_percent: 0.8,
                activation_percent: 0.15,
                giveback_percent: 0.1,
                minimum_lock_percent: 0.05,
                trailing_activation_percent: None,
                trailing_distance_percent: None,
                breakeven_activation_percent: None,
            },
            actual_exit_net_return_percent: 0.05,
            exit_reason: "profit-protection".into(),
            path: vec![
                Quote {
                    timestamp_ms: 2,
                    bid: 101.0,
                    ask: 101.2,
                },
                Quote {
                    timestamp_ms: 3,
                    bid: 101.0,
                    ask: 101.2,
                },
            ],
        };
        let result = replay_net_return(&sample, sample.baseline).unwrap();
        assert!((result - 0.799).abs() < 1e-9);
    }

    #[test]
    fn profile_scoped_trailing_and_breakeven_candidates_use_executable_paths() {
        let mut sample = parsed_synthetic(PathCase::TrainingAndHoldoutImprove).remove(0);
        sample.direction = 1.0;
        sample.entry_price = 100.0;
        sample.entry_fees = 0.0;
        sample.counterfactual_exit_fee_attribution =
            quote_currency_fee_attribution(0.0, &[10, 20, 30, 40]);
        sample.stop_loss_percent = 0.5;
        sample.take_profit_percent = 20.0;
        sample.baseline = ProfitProtectionParameters {
            stop_loss_percent: 0.5,
            take_profit_percent: 20.0,
            activation_percent: f64::MAX,
            giveback_percent: 0.3,
            minimum_lock_percent: 0.0,
            trailing_activation_percent: None,
            trailing_distance_percent: None,
            breakeven_activation_percent: None,
        };
        sample.path = vec![
            Quote {
                timestamp_ms: 10,
                bid: 100.0,
                ask: 100.0,
            },
            Quote {
                timestamp_ms: 20,
                bid: 102.0,
                ask: 102.0,
            },
            Quote {
                timestamp_ms: 30,
                bid: 101.5,
                ask: 101.5,
            },
            Quote {
                timestamp_ms: 40,
                bid: 99.8,
                ask: 99.8,
            },
        ];

        let mut trailing = sample.baseline;
        trailing.trailing_activation_percent = Some(0.5);
        trailing.trailing_distance_percent = Some(0.3);
        let trailing_exit = replay_outcome(&sample, trailing).unwrap();
        assert_eq!(trailing_exit.exit_reason, ReplayExitReason::TrailingStop);
        assert_eq!(trailing_exit.exit_timestamp_ms, 30);

        let mut breakeven = sample.baseline;
        breakeven.breakeven_activation_percent = Some(0.5);
        let breakeven_exit = replay_outcome(&sample, breakeven).unwrap();
        assert_eq!(
            breakeven_exit.exit_reason,
            ReplayExitReason::BreakevenProtection
        );
        assert_eq!(breakeven_exit.exit_timestamp_ms, 40);

        let candidates = empirical_policy_candidates(std::slice::from_ref(&sample));
        assert!(
            !candidates
                .values(ExitPolicyDimension::TrailingActivation)
                .is_empty()
        );
        assert!(
            !candidates
                .values(ExitPolicyDimension::TrailingDistance)
                .is_empty()
        );
        assert!(
            !candidates
                .values(ExitPolicyDimension::BreakevenActivation)
                .is_empty()
        );
        assert!(valid_policy_parameters(trailing));
        assert!(!valid_policy_parameters(ProfitProtectionParameters {
            trailing_activation_percent: Some(0.2),
            ..sample.baseline
        }));
        assert!(!valid_policy_parameters(ProfitProtectionParameters {
            breakeven_activation_percent: Some(MAX_EXIT_PARAMETER_PERCENT + 0.1),
            ..sample.baseline
        }));
    }

    #[test]
    fn short_replay_exits_at_ask() {
        let sample = ReplayObservation {
            event_id: "short-1".into(),
            exit_timestamp_ms: 1,
            observed_at_ms: 3,
            direction: -1.0,
            entry_price: 100.0,
            quantity: 1.0,
            entry_fees: 0.1,
            initial_risk_quote: None,
            initial_risk_currency: None,
            quote_currency: None,
            market_regime: None,
            entry_timestamp_ms: None,
            maximum_holding_duration_days: None,
            profile_id: None,
            profile_version: None,
            initial_entry_notional_quote: None,
            entry_fill_timing: None,
            execution_costs_complete: false,
            counterfactual_exit_fee_attribution: quote_currency_fee_attribution(0.001, &[2]),
            gross_executable_mfe_percent: 0.0,
            gross_executable_mae_percent: 0.0,
            net_executable_mfe_percent: 0.0,
            net_executable_mae_percent: 0.0,
            measured_slippage_quote: None,
            measured_slippage_currency: None,
            independent_reconciliation_complete: false,
            stop_loss_percent: 5.0,
            take_profit_percent: 5.0,
            baseline: ProfitProtectionParameters {
                stop_loss_percent: 0.5,
                take_profit_percent: 0.8,
                activation_percent: 0.15,
                giveback_percent: 0.1,
                minimum_lock_percent: 0.05,
                trailing_activation_percent: None,
                trailing_distance_percent: None,
                breakeven_activation_percent: None,
            },
            actual_exit_net_return_percent: 0.05,
            exit_reason: "profit-protection".into(),
            path: vec![Quote {
                timestamp_ms: 2,
                bid: 98.8,
                ask: 99.0,
            }],
        };
        let result = replay_net_return(&sample, sample.baseline).unwrap();
        assert!((result - 0.801).abs() < 1e-9);
    }

    #[test]
    fn stop_loss_threshold_includes_entry_and_exit_fees() {
        let sample = ReplayObservation {
            event_id: "fee-stop-loss".into(),
            exit_timestamp_ms: 1,
            observed_at_ms: 3,
            direction: 1.0,
            entry_price: 100.0,
            quantity: 1.0,
            entry_fees: 0.1,
            initial_risk_quote: None,
            initial_risk_currency: None,
            quote_currency: None,
            market_regime: None,
            entry_timestamp_ms: None,
            maximum_holding_duration_days: None,
            profile_id: None,
            profile_version: None,
            initial_entry_notional_quote: None,
            entry_fill_timing: None,
            execution_costs_complete: false,
            counterfactual_exit_fee_attribution: quote_currency_fee_attribution(0.001, &[2, 3]),
            gross_executable_mfe_percent: 0.0,
            gross_executable_mae_percent: 0.0,
            net_executable_mfe_percent: 0.0,
            net_executable_mae_percent: 0.0,
            measured_slippage_quote: None,
            measured_slippage_currency: None,
            independent_reconciliation_complete: false,
            stop_loss_percent: 0.65,
            take_profit_percent: 5.0,
            baseline: ProfitProtectionParameters {
                stop_loss_percent: 0.5,
                take_profit_percent: 0.8,
                activation_percent: 0.15,
                giveback_percent: 0.1,
                minimum_lock_percent: 0.05,
                trailing_activation_percent: None,
                trailing_distance_percent: None,
                breakeven_activation_percent: None,
            },
            actual_exit_net_return_percent: -0.65,
            exit_reason: "stop-loss".into(),
            path: vec![
                Quote {
                    timestamp_ms: 2,
                    bid: 99.4,
                    ask: 99.6,
                },
                Quote {
                    timestamp_ms: 3,
                    bid: 100.0,
                    ask: 100.2,
                },
            ],
        };
        // Gross loss is only 0.6%, but net loss is 0.7994% after both fees,
        // so the 0.65% net stop must trigger at this sampled executable bid.
        let result = replay_net_return(&sample, sample.baseline).unwrap();
        assert!((result + 0.7994).abs() < 1e-9);
    }

    #[test]
    fn take_profit_threshold_includes_entry_and_exit_fees() {
        let sample = ReplayObservation {
            event_id: "fee-take-profit".into(),
            exit_timestamp_ms: 1,
            observed_at_ms: 3,
            direction: 1.0,
            entry_price: 100.0,
            quantity: 1.0,
            entry_fees: 0.1,
            initial_risk_quote: None,
            initial_risk_currency: None,
            quote_currency: None,
            market_regime: None,
            entry_timestamp_ms: None,
            maximum_holding_duration_days: None,
            profile_id: None,
            profile_version: None,
            initial_entry_notional_quote: None,
            entry_fill_timing: None,
            execution_costs_complete: false,
            counterfactual_exit_fee_attribution: quote_currency_fee_attribution(0.001, &[2, 3]),
            gross_executable_mfe_percent: 0.0,
            gross_executable_mae_percent: 0.0,
            net_executable_mfe_percent: 0.0,
            net_executable_mae_percent: 0.0,
            measured_slippage_quote: None,
            measured_slippage_currency: None,
            independent_reconciliation_complete: false,
            stop_loss_percent: 5.0,
            take_profit_percent: 0.5,
            baseline: ProfitProtectionParameters {
                stop_loss_percent: 0.5,
                take_profit_percent: 0.8,
                activation_percent: 0.15,
                giveback_percent: 0.1,
                minimum_lock_percent: 0.05,
                trailing_activation_percent: None,
                trailing_distance_percent: None,
                breakeven_activation_percent: None,
            },
            actual_exit_net_return_percent: 0.5,
            exit_reason: "take-profit".into(),
            path: vec![
                Quote {
                    timestamp_ms: 2,
                    bid: 100.6,
                    ask: 100.8,
                },
                Quote {
                    timestamp_ms: 3,
                    bid: 100.8,
                    ask: 101.0,
                },
            ],
        };
        // The first quote is +0.6% gross but only +0.3994% net, so a 0.5%
        // take-profit threshold must wait for the second executable quote.
        let result = replay_net_return(&sample, sample.baseline).unwrap();
        assert!((result - 0.5992).abs() < 1e-9);
    }

    #[test]
    fn rejects_future_evidence_and_missing_baseline_parameters() {
        let value = serde_json::json!({"symbol":"BTC-USDT", "strategy":"live-momentum", "exit_timestamp_ms":10, "observed_at_ms":30});
        assert!(parse_observation(&value, "BTC-USDT", "live-momentum", 29).is_none());
        assert!(parse_observation(&value, "BTC-USDT", "live-momentum", 30).is_none());
    }

    #[test]
    fn missing_quote_fee_identity_or_complete_path_is_not_imputed() {
        let mut observations = synthetic_observations(PathCase::TrainingAndHoldoutImprove);
        observations[0]["entry_context"]["entry_fee_currencies"] =
            Value::Array(vec![Value::String("BNB".into())]);
        assert!(
            parse_observation(&observations[0], "BTC-USDT", "live-momentum", u64::MAX).is_none()
        );
        observations[0]["entry_context"]["entry_fee_currencies"] = serde_json::json!(["USDT"]);
        observations[0]["price_path_evidence"]["complete"] = Value::Bool(false);
        assert!(
            parse_observation(&observations[0], "BTC-USDT", "live-momentum", u64::MAX).is_none()
        );
    }

    #[test]
    fn uses_bounded_candidate_grid() {
        assert_eq!(
            candidate_values(0.0, 10.0, 0.01).len(),
            MAX_CANDIDATES_PER_PARAMETER
        );
        assert_eq!(candidate_values(f64::NAN, 1.0, 0.1).len(), 0);
    }

    #[test]
    fn rejects_invalid_fees_exit_thresholds_and_protection_baselines() {
        let baseline = ProfitProtectionParameters {
            stop_loss_percent: 0.5,
            take_profit_percent: 0.8,
            activation_percent: 0.15,
            giveback_percent: 0.1,
            minimum_lock_percent: 0.05,
            trailing_activation_percent: None,
            trailing_distance_percent: None,
            breakeven_activation_percent: None,
        };
        assert!(valid_exit_policy_inputs(0.5, 0.8, baseline));
        assert!(valid_counterfactual_exit_fee_rate(0.001));
        assert!(!valid_counterfactual_exit_fee_rate(-0.001));
        assert!(!valid_counterfactual_exit_fee_rate(0.1001));
        assert!(!valid_exit_policy_inputs(0.0, 0.8, baseline));
        assert!(!valid_exit_policy_inputs(0.5, -0.1, baseline));
        assert!(!valid_exit_policy_inputs(
            0.5,
            0.8,
            ProfitProtectionParameters {
                minimum_lock_percent: 0.2,
                ..baseline
            }
        ));
    }

    #[test]
    fn paired_downside_gate_rejects_mean_winners_with_damaging_tail_losses() {
        assert!(passes_downside_gate(0.4, -0.1, -0.2));
        assert!(!passes_downside_gate(0.7, -0.1, -0.2));
        assert!(!passes_downside_gate(0.4, -0.3, -0.3));
        assert!(!passes_downside_gate(0.4, -0.1, -0.6));

        let mut differences = vec![-0.31, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.9];
        differences.sort_by(f64::total_cmp);
        assert_eq!(nearest_rank_percentile(&differences, 0.10), Some(-0.31));
        assert_eq!(nearest_rank_percentile(&differences, 0.50), Some(0.2));
    }

    #[test]
    fn chronological_replay_assesses_bounded_policy_when_later_net_holdout_improves() {
        let observations = synthetic_observations(PathCase::TrainingAndHoldoutImprove);
        let evidence =
            select_exit_policy(observations.iter(), "BTC-USDT", "live-momentum", u64::MAX)
                .expect("complete improving chronological holdout should be assessed");
        assert_eq!(evidence.sample_count, 40);
        assert_eq!(evidence.training_count, 30);
        assert_eq!(evidence.validation_count, 10);
        assert!(evidence.validation_gain_percent >= MIN_VALIDATION_GAIN_PERCENT);
        assert_eq!(
            evidence.as_of_ms,
            10_000_000 + 39 * 10_000_000 + 30 * 60_000
        );
        assert!(!evidence.metrics.promotion_eligible);
        assert!(
            evidence
                .metrics
                .promotion_blockers
                .contains(&"measured_execution_cost_coverage_incomplete".to_owned())
        );
        assert!(
            evidence
                .metrics
                .values
                .contains_key("capital_efficiency_quote_pnl_per_quote_capital_day")
        );
        assert_eq!(
            evidence.metrics.coverage["capital_efficiency_quote_pnl_per_quote_capital_day"].status,
            strategy::ExitPolicyMetricCoverageStatus::Partial
        );
        assert!(
            !evidence
                .metrics
                .promotion_blockers
                .contains(&"capital_efficiency_fill_timing_coverage_incomplete".to_owned())
        );
        assert!(
            evidence
                .metrics
                .promotion_blockers
                .contains(&"sourced_timestamped_fx_unavailable".to_owned())
        );
        for metric in [
            "net_expectancy_percent",
            "expectancy_r",
            "profit_factor",
            "payoff_ratio",
            "average_winner_percent",
            "average_loser_percent",
            "winner_preservation_rate",
            "loser_recovery_rate",
            "max_trade_sequence_drawdown_percent_points",
            "mean_to_sample_standard_deviation",
            "mean_position_duration_ms",
            "capital_efficiency_quote_pnl_per_quote_capital_day",
            "modeled_fee_burden_percent",
            "worst_market_regime_improvement_percent",
            "worst_direction_improvement_percent",
            "independently_reconciled_sample_coverage_percent",
        ] {
            assert!(evidence.metrics.values.contains_key(metric), "{metric}");
            assert!(evidence.metrics.coverage.contains_key(metric), "{metric}");
        }
        assert_eq!(
            evidence.metrics.coverage["expectancy_r"].source_fields[0],
            "entry_context.initial_risk_reference.schema_version"
        );
        assert_eq!(
            evidence.metrics.values["independently_reconciled_sample_coverage_percent"],
            100.0
        );
        assert!(
            !evidence
                .metrics
                .values
                .contains_key("measured_slippage_percent_of_notional")
        );
        assert_eq!(
            evidence.metrics.coverage["measured_slippage_percent_of_notional"].status,
            strategy::ExitPolicyMetricCoverageStatus::Unavailable
        );
        assert_eq!(
            evidence.metrics.coverage["capital_efficiency_quote_pnl_per_quote_capital_day"].status,
            strategy::ExitPolicyMetricCoverageStatus::Partial
        );
        assert_eq!(
            evidence.metrics.coverage["account_currency_expectancy_percent"].status,
            strategy::ExitPolicyMetricCoverageStatus::Unavailable
        );

        let specs = strategy::Strategy::from_name("live-momentum")
            .unwrap()
            .parameter_specs();
        for (key, value) in [
            (ACTIVATION_KEY, evidence.parameters.activation_percent),
            (GIVEBACK_KEY, evidence.parameters.giveback_percent),
            (MINIMUM_LOCK_KEY, evidence.parameters.minimum_lock_percent),
        ] {
            let spec = specs.iter().find(|spec| spec.key.as_str() == key).unwrap();
            assert!(
                (spec.min..=spec.max).contains(&value),
                "{key} escaped catalog bounds"
            );
        }

        let samples = parsed_synthetic(PathCase::TrainingAndHoldoutImprove);
        let report = report_replay(
            &samples[30..],
            evidence.parameters,
            evidence
                .maximum_holding_duration_ms
                .map(|duration_ms| duration_ms as f64 / 86_400_000.0),
        );
        assert!(report.selected_mean_net_percent > report.baseline_mean_net_percent);
        assert_eq!(report.actual_exit_reason_counts, [8, 0, 2]);
        assert!(report.hold_improvement_mean_percent > 0.0);
        assert!(report.hold_improvement_median_percent > 0.0);
        assert_eq!(report.hold_improvement_positive_rate, 0.6);
        assert_eq!(report.baseline_replay_exit_reason_counts[3..5], [8, 0]);
        assert_eq!(
            report.selected_replay_exit_reason_counts,
            [2, 0, 0, 2, 0, 6, 0]
        );
        assert!(report.selected_max_drawdown_percent <= report.baseline_max_drawdown_percent);
        for direction in [&report.long_holdout, &report.short_holdout] {
            assert_eq!(direction.sample_count, 5);
            assert!(direction.selected_mean_net_percent > direction.baseline_mean_net_percent);
            assert!(direction.hold_improvement_mean_percent > 0.0);
            assert!(direction.hold_improvement_median_percent >= 0.0);
            assert_eq!(direction.hold_improvement_positive_rate, 0.6);
            assert_eq!(direction.actual_exit_reason_counts, [4, 0, 1]);
        }
    }

    #[test]
    fn incomplete_risk_cost_or_regime_evidence_reports_blockers_without_promotion() {
        let mut observations = synthetic_observations(PathCase::TrainingAndHoldoutImprove);
        for value in &mut observations {
            value["entry_context"]
                .as_object_mut()
                .unwrap()
                .remove("initial_risk_reference");
            value.as_object_mut().unwrap().remove("market_regime");
            value
                .as_object_mut()
                .unwrap()
                .remove("independent_reconciliation");
        }
        let evidence =
            select_exit_policy(observations.iter(), "BTC-USDT", "live-momentum", u64::MAX)
                .expect("net replay remains assessable even though evidence blocks promotion");
        assert!(!evidence.metrics.promotion_eligible);
        assert!(
            evidence
                .metrics
                .promotion_blockers
                .contains(&"authoritative_initial_risk_unavailable".to_owned())
        );
        assert!(
            evidence
                .metrics
                .promotion_blockers
                .contains(&"measured_execution_cost_coverage_incomplete".to_owned())
        );
        assert!(
            evidence
                .metrics
                .promotion_blockers
                .contains(&"market_regime_robustness_unavailable".to_owned())
        );
        assert!(
            evidence
                .metrics
                .promotion_blockers
                .contains(&"independent_settlement_reconciliation_unavailable".to_owned())
        );
        assert!(!evidence.metrics.values.contains_key("expectancy_r"));
        assert_eq!(
            evidence.metrics.coverage["expectancy_r"].status,
            strategy::ExitPolicyMetricCoverageStatus::Unavailable
        );
        assert!(
            !evidence
                .metrics
                .values
                .contains_key("account_currency_expectancy_percent")
        );
        assert!(
            !evidence
                .metrics
                .values
                .contains_key("measured_slippage_percent_of_notional")
        );
    }

    #[test]
    fn chronological_replay_reports_candidates_rejected_by_holdout_gates() {
        for case in [PathCase::HoldoutReverses, PathCase::HoldoutFlat] {
            let observations = synthetic_observations(case);
            let assessment =
                select_exit_policy(observations.iter(), "BTC-USDT", "live-momentum", u64::MAX)
                    .expect("adequate evidence should preserve rejected candidate assessment");
            assert!(!assessment.metrics.promotion_eligible);
            assert!(
                assessment
                    .metrics
                    .promotion_blockers
                    .iter()
                    .any(|blocker| blocker.starts_with("validation_"))
            );
        }
    }

    #[test]
    fn chronological_replay_requires_adequate_post_exit_path_coverage() {
        let mut observations = synthetic_observations(PathCase::TrainingAndHoldoutImprove);
        for value in &mut observations {
            value["price_path"] =
                serde_json::Value::Array(value["price_path"].as_array().unwrap()[..8].to_vec());
            value["price_path_evidence"]["complete"] = Value::Bool(false);
        }
        assert!(
            select_exit_policy(observations.iter(), "BTC-USDT", "live-momentum", u64::MAX)
                .is_none()
        );
        let assessment = unavailable_exit_policy_assessment(
            observations.iter(),
            "BTC-USDT",
            "live-momentum",
            u64::MAX,
        );
        assert_eq!(assessment.status, "insufficient_evidence");
        assert_eq!(assessment.stop_loss_percent, None);
        assert_eq!(
            assessment.metrics.values["executable_path_source_coverage_percent"],
            0.0
        );
        assert!(
            !assessment
                .metrics
                .values
                .contains_key("net_expectancy_percent")
        );
        assert_eq!(
            assessment.metrics.coverage["net_expectancy_percent"].status,
            strategy::ExitPolicyMetricCoverageStatus::Unavailable
        );
        assert!(
            assessment
                .metrics
                .promotion_blockers
                .contains(&"insufficient_complete_executable_path_observations".to_owned())
        );
    }

    #[test]
    fn horizon_coverage_rejects_sparse_or_truncated_paths() {
        let complete = (0..=30)
            .map(|minute| Quote {
                timestamp_ms: minute * 60_000,
                bid: 100.0,
                ask: 100.1,
            })
            .collect::<Vec<_>>();
        assert!(has_horizon_coverage(&complete, 0, 30 * 60_000));
        assert!(!has_horizon_coverage(&complete[..2], 0, 30 * 60_000));
        let truncated = complete[..20].to_vec();
        assert!(!has_horizon_coverage(&truncated, 0, 30 * 60_000));
        let large_gap = complete
            .iter()
            .filter(|quote| !(10 * 60_000..=20 * 60_000).contains(&quote.timestamp_ms))
            .copied()
            .collect::<Vec<_>>();
        assert!(!has_horizon_coverage(&large_gap, 0, 30 * 60_000));
    }

    #[test]
    fn maximum_hold_candidates_require_captured_duration_and_stay_inside_profile_bounds() {
        let mut samples = parsed_synthetic(PathCase::TrainingAndHoldoutImprove);
        for sample in &mut samples {
            sample.maximum_holding_duration_days = Some(5.0);
        }
        let candidates = maximum_hold_duration_candidates(
            &samples[..MIN_TRAIN_OBSERVATIONS],
            "rsi2-oversold-rebound",
        );
        assert_eq!(candidates.len(), 15);
        assert_eq!(candidates.first(), Some(&1.0));
        assert_eq!(candidates.last(), Some(&15.0));

        samples[0].maximum_holding_duration_days = None;
        assert!(
            maximum_hold_duration_candidates(
                &samples[..MIN_TRAIN_OBSERVATIONS],
                "rsi2-oversold-rebound"
            )
            .is_empty()
        );
    }

    #[test]
    fn profile_replay_requires_exact_consistent_identity_and_version() {
        let mut observation = serde_json::json!({
            "exit_parameter_provenance": {"profile_id":"profile-v1", "profile_version":7},
            "entry_context": {"strategy_decision":{"learning_profile_id":"profile-v1", "learning_profile_version":7}}
        });
        assert!(observation_matches_profile_version(
            &observation,
            "profile-v1",
            7
        ));
        assert!(!observation_matches_profile_version(
            &observation,
            "profile-v2",
            7
        ));
        observation["exit_parameter_provenance"]["profile_version"] = serde_json::json!(8);
        assert!(!observation_matches_profile_version(
            &observation,
            "profile-v1",
            7
        ));
    }
}
