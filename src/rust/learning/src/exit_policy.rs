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
use std::collections::BTreeSet;

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
const MAX_CONFIGURED_TAKER_FEE_RATE: f64 = 0.1;

const ACTIVATION_KEY: &str = "profit_protection_activation_percent";
const GIVEBACK_KEY: &str = "profit_protection_giveback_percent";
const MINIMUM_LOCK_KEY: &str = "profit_protection_minimum_lock_percent";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ProfitProtectionParameters {
    pub stop_loss_percent: f64,
    pub take_profit_percent: f64,
    pub activation_percent: f64,
    pub giveback_percent: f64,
    pub minimum_lock_percent: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ExitPolicyEvidence {
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
    taker_fee_rate: f64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    baseline: ProfitProtectionParameters,
    #[allow(dead_code)]
    actual_exit_net_return_percent: f64,
    #[allow(dead_code)]
    exit_reason: String,
    path: Vec<Quote>,
}

#[derive(Clone, Copy, Debug)]
struct Quote {
    timestamp_ms: u64,
    bid: f64,
    ask: f64,
}

#[derive(Clone, Copy, Debug)]
struct CandidateScore {
    parameters: ProfitProtectionParameters,
    training_mean: f64,
}

#[derive(Clone, Copy)]
enum ExitPolicyDimension {
    StopLoss,
    TakeProfit,
    Activation,
    Giveback,
    MinimumLock,
}

impl ExitPolicyDimension {
    const ALL: [Self; 5] = [
        Self::StopLoss,
        Self::TakeProfit,
        Self::Activation,
        Self::Giveback,
        Self::MinimumLock,
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
        }
    }
}

struct EmpiricalPolicyCandidates([Vec<f64>; 5]);

impl EmpiricalPolicyCandidates {
    fn values(&self, dimension: ExitPolicyDimension) -> &[f64] {
        match dimension {
            ExitPolicyDimension::StopLoss => &self.0[0],
            ExitPolicyDimension::TakeProfit => &self.0[1],
            ExitPolicyDimension::Activation => &self.0[2],
            ExitPolicyDimension::Giveback => &self.0[3],
            ExitPolicyDimension::MinimumLock => &self.0[4],
        }
    }
}

fn median_baseline(samples: &[ReplayObservation]) -> Option<ProfitProtectionParameters> {
    let median = |mut values: Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values.get(values.len() / 2).copied()
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
    })
}

fn empirical_policy_candidates(samples: &[ReplayObservation]) -> EmpiricalPolicyCandidates {
    let mut values: [Vec<f64>; 5] = std::array::from_fn(|_| Vec::new());
    for sample in samples {
        values[0].push(sample.stop_loss_percent);
        values[1].push(sample.take_profit_percent);
        values[2].push(sample.baseline.activation_percent);
        values[3].push(sample.baseline.giveback_percent);
        values[4].push(sample.baseline.minimum_lock_percent);

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
            }
            let giveback = peak - net;
            if giveback > 0.0 {
                values[3].push(giveback);
            }
        }
    }
    for (index, candidates) in values.iter_mut().enumerate() {
        candidates.retain(|value| {
            value.is_finite()
                && if index == 4 {
                    *value >= 0.0
                } else {
                    *value > 0.0
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
}

/// Select an empirically derived rule only when later chronological
/// validation beats the captured baseline. `cutoff_timestamp_ms` is the
/// point-in-time boundary for the caller's profile snapshot.
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
    let mut best = CandidateScore {
        parameters: baseline_parameters,
        training_mean: score(training, baseline_parameters)?,
    };
    let candidates = empirical_policy_candidates(training);
    let mut improved = true;
    while improved {
        improved = false;
        for dimension in ExitPolicyDimension::ALL {
            for value in candidates.values(dimension) {
                let mut parameters = best.parameters;
                parameters.set(dimension, *value);
                if !valid_policy_parameters(parameters) {
                    continue;
                }
                let Some(candidate_mean) = score(training, parameters) else {
                    continue;
                };
                if candidate_mean > best.training_mean + f64::EPSILON {
                    best = CandidateScore {
                        parameters,
                        training_mean: candidate_mean,
                    };
                    improved = true;
                }
            }
        }
    }

    // Require an in-sample gain over the per-position captured baseline before
    // considering the untouched chronological validation block.
    if best.training_mean <= historical_baseline_score {
        return None;
    }

    // Require paired validation improvement and a one-sided 95% lower bound
    // above the baseline. A positive mean alone is not enough to promote.
    let paired = validation
        .iter()
        .filter_map(|sample| {
            Some((
                replay_net_return(sample, sample.baseline)?,
                replay_net_return(sample, best.parameters)?,
            ))
        })
        .collect::<Vec<_>>();
    if paired.len() < MIN_VALIDATION_OBSERVATIONS {
        return None;
    }
    let baseline_mean = paired.iter().map(|pair| pair.0).sum::<f64>() / paired.len() as f64;
    let selected_mean = paired.iter().map(|pair| pair.1).sum::<f64>() / paired.len() as f64;
    let differences = paired
        .iter()
        .map(|pair| pair.1 - pair.0)
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
    if gain < MIN_VALIDATION_GAIN_PERCENT
        || lower_bound <= 0.0
        || selected_mean < baseline_mean - MAX_VALIDATION_LOSS_PERCENT
        || !passes_downside_gate(
            negative_difference_rate,
            tenth_percentile_difference,
            worst_difference,
        )
    {
        return None;
    }
    Some(ExitPolicyEvidence {
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
    })
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
    let entry_price = entry["average_price"].as_f64()?;
    let quantity = entry["quantity"].as_f64()?;
    let entry_fees = entry["fees"].as_f64()?;
    let direction = match value["side"].as_str()? {
        "sell" => 1.0,
        "buy" => -1.0,
        _ => return None,
    };
    let taker_fee_rate = value["counterfactual_exit_fee_rate"].as_f64()?;
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
    };
    if ![
        entry_price,
        quantity,
        entry_fees,
        taker_fee_rate,
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
        || !valid_exit_policy_inputs(
            taker_fee_rate,
            stop_loss_percent,
            take_profit_percent,
            baseline,
        )
    {
        return None;
    }
    let post_exit_path = value["price_path"]
        .as_array()?
        .iter()
        .filter_map(parse_quote)
        .filter(|quote| quote.timestamp_ms >= exit_timestamp_ms)
        .collect::<Vec<_>>();
    if !has_horizon_coverage(&post_exit_path, exit_timestamp_ms, horizon_ms) {
        return None;
    }
    let mut path = value["pre_exit_price_path"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(parse_quote)
        .collect::<Vec<_>>();
    path.extend(post_exit_path);
    path.sort_by_key(|quote| quote.timestamp_ms);
    path.dedup_by_key(|quote| quote.timestamp_ms);
    if path.len() < 2 || path.last()?.timestamp_ms > observed_at_ms {
        return None;
    }
    let actual_exit_net_return_percent = value["exit_return_percent"].as_f64()?;
    if !actual_exit_net_return_percent.is_finite() {
        return None;
    }
    Some(ReplayObservation {
        event_id: event_id.to_owned(),
        exit_timestamp_ms,
        observed_at_ms,
        direction,
        entry_price,
        quantity,
        entry_fees,
        taker_fee_rate,
        stop_loss_percent,
        take_profit_percent,
        baseline,
        actual_exit_net_return_percent,
        exit_reason: value["reason"].as_str()?.to_owned(),
        path,
    })
}

fn valid_exit_policy_inputs(
    taker_fee_rate: f64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    baseline: ProfitProtectionParameters,
) -> bool {
    taker_fee_rate.is_finite()
        && (0.0..=MAX_CONFIGURED_TAKER_FEE_RATE).contains(&taker_fee_rate)
        && stop_loss_percent.is_finite()
        && stop_loss_percent > 0.0
        && take_profit_percent.is_finite()
        && take_profit_percent > 0.0
        && valid_policy_parameters(baseline)
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

fn replay_net_return(
    sample: &ReplayObservation,
    parameters: ProfitProtectionParameters,
) -> Option<f64> {
    replay_outcome(sample, parameters).map(|outcome| outcome.net_return_percent)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplayExitReason {
    StopLoss,
    ProfitProtection,
    TakeProfit,
    Horizon,
}

#[derive(Clone, Copy, Debug)]
struct ReplayOutcome {
    net_return_percent: f64,
    #[allow(dead_code)]
    exit_reason: ReplayExitReason,
}

fn replay_outcome(
    sample: &ReplayObservation,
    parameters: ProfitProtectionParameters,
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
            exit = Some((executable_price, ReplayExitReason::StopLoss));
            break;
        }
        if peak >= parameters.activation_percent
            && net_percent
                <= (peak - parameters.giveback_percent).max(parameters.minimum_lock_percent)
        {
            exit = Some((executable_price, ReplayExitReason::ProfitProtection));
            break;
        }
        if net_percent >= parameters.take_profit_percent {
            exit = Some((executable_price, ReplayExitReason::TakeProfit));
            break;
        }
    }
    let (price, exit_reason) = match exit {
        Some(exit) => exit,
        None => {
            let quote = sample.path.last()?;
            (
                if sample.direction > 0.0 {
                    quote.bid
                } else {
                    quote.ask
                },
                ReplayExitReason::Horizon,
            )
        }
    };
    Some(ReplayOutcome {
        net_return_percent: ((price - sample.entry_price) * sample.quantity * sample.direction
            - sample.entry_fees
            - price * sample.quantity * sample.taker_fee_rate)
            / notional
            * 100.0,
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
    let net_percent =
        ((executable_price - sample.entry_price) * sample.quantity * sample.direction
            - sample.entry_fees
            - executable_price * sample.quantity * sample.taker_fee_rate)
            / notional
            * 100.0;
    net_percent.is_finite().then_some(net_percent)
}

fn score(samples: &[ReplayObservation], parameters: ProfitProtectionParameters) -> Option<f64> {
    let returns = samples
        .iter()
        .map(|sample| replay_net_return(sample, parameters))
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

    #[derive(Debug)]
    struct ReplayReport {
        baseline_mean_net_percent: f64,
        selected_mean_net_percent: f64,
        baseline_max_drawdown_percent: f64,
        selected_max_drawdown_percent: f64,
        actual_exit_reason_counts: [usize; 3],
        baseline_replay_exit_reason_counts: [usize; 4],
        selected_replay_exit_reason_counts: [usize; 4],
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
    ) -> ReplayReport {
        let baseline = samples
            .iter()
            .map(|sample| replay_outcome(sample, sample.baseline).unwrap())
            .collect::<Vec<_>>();
        let candidate = samples
            .iter()
            .map(|sample| replay_outcome(sample, selected).unwrap())
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
            ),
            short_holdout: direction_summary(
                &samples
                    .iter()
                    .filter(|sample| sample.direction < 0.0)
                    .cloned()
                    .collect::<Vec<_>>(),
                selected,
            ),
        }
    }

    fn direction_summary(
        samples: &[ReplayObservation],
        selected: ProfitProtectionParameters,
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
            .map(|sample| replay_outcome(sample, selected).unwrap().net_return_percent)
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

    fn max_drawdown(returns: &[f64]) -> f64 {
        let (mut cumulative, mut peak, mut drawdown) = (0.0_f64, 0.0_f64, 0.0_f64);
        for value in returns {
            cumulative += value;
            peak = peak.max(cumulative);
            drawdown = drawdown.max(peak - cumulative);
        }
        drawdown
    }

    fn reason_counts(outcomes: &[ReplayOutcome]) -> [usize; 4] {
        let mut counts = [0; 4];
        for outcome in outcomes {
            counts[match outcome.exit_reason {
                ReplayExitReason::StopLoss => 0,
                ReplayExitReason::ProfitProtection => 1,
                ReplayExitReason::TakeProfit => 2,
                ReplayExitReason::Horizon => 3,
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
                let timestamp = 1_000_000 + index as u64 * 10_000_000;
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
                    3 => vec![100.10, 100.50, 101.00, 101.00, 101.00, 101.00, 101.00],
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
                    3 => ("take-profit", 0.89),
                    _ => ("stop-loss", -0.81),
                };
                serde_json::json!({
                    "event_id": format!("synthetic-{index}"),
                    "symbol": "BTC-USDT",
                    "strategy": "live-momentum",
                    "reason": reason,
                    "side": if is_long { "sell" } else { "buy" },
                    "exit_timestamp_ms": timestamp,
                    "observed_at_ms": timestamp + 30 * 60_000,
                    "horizon_ms": 30 * 60_000,
                    "entry_context": {"average_price": 100.0, "quantity": 1.0, "fees": 0.01},
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
                })
            })
            .collect()
    }

    fn parsed_synthetic(case: PathCase) -> Vec<ReplayObservation> {
        let specs = strategy::Strategy::from_name("live-momentum")
            .unwrap()
            .parameter_specs();
        synthetic_observations(case)
            .iter()
            .filter_map(|value| {
                parse_observation(value, "BTC-USDT", "live-momentum", u64::MAX, &specs)
            })
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
            taker_fee_rate: 0.001,
            stop_loss_percent: 5.0,
            take_profit_percent: 5.0,
            baseline: ProfitProtectionParameters {
                stop_loss_percent: 0.5,
                take_profit_percent: 0.8,
                activation_percent: 0.15,
                giveback_percent: 0.1,
                minimum_lock_percent: 0.05,
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
    fn short_replay_exits_at_ask() {
        let sample = ReplayObservation {
            event_id: "short-1".into(),
            exit_timestamp_ms: 1,
            observed_at_ms: 3,
            direction: -1.0,
            entry_price: 100.0,
            quantity: 1.0,
            entry_fees: 0.1,
            taker_fee_rate: 0.001,
            stop_loss_percent: 5.0,
            take_profit_percent: 5.0,
            baseline: ProfitProtectionParameters {
                stop_loss_percent: 0.5,
                take_profit_percent: 0.8,
                activation_percent: 0.15,
                giveback_percent: 0.1,
                minimum_lock_percent: 0.05,
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
            taker_fee_rate: 0.001,
            stop_loss_percent: 0.65,
            take_profit_percent: 5.0,
            baseline: ProfitProtectionParameters {
                stop_loss_percent: 0.5,
                take_profit_percent: 0.8,
                activation_percent: 0.15,
                giveback_percent: 0.1,
                minimum_lock_percent: 0.05,
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
            taker_fee_rate: 0.001,
            stop_loss_percent: 5.0,
            take_profit_percent: 0.5,
            baseline: ProfitProtectionParameters {
                stop_loss_percent: 0.5,
                take_profit_percent: 0.8,
                activation_percent: 0.15,
                giveback_percent: 0.1,
                minimum_lock_percent: 0.05,
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
        assert!(parse_observation(&value, "BTC-USDT", "live-momentum", 29, &[]).is_none());
        assert!(parse_observation(&value, "BTC-USDT", "live-momentum", 30, &[]).is_none());
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
        };
        assert!(valid_exit_policy_inputs(0.001, 0.5, 0.8, baseline));
        assert!(!valid_exit_policy_inputs(-0.001, 0.5, 0.8, baseline));
        assert!(!valid_exit_policy_inputs(0.1001, 0.5, 0.8, baseline));
        assert!(!valid_exit_policy_inputs(0.001, 0.0, 0.8, baseline));
        assert!(!valid_exit_policy_inputs(0.001, 0.5, -0.1, baseline));
        assert!(!valid_exit_policy_inputs(
            0.001,
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
    fn chronological_replay_promotes_bounded_policy_when_later_net_holdout_improves() {
        let observations = synthetic_observations(PathCase::TrainingAndHoldoutImprove);
        let evidence =
            select_exit_policy(observations.iter(), "BTC-USDT", "live-momentum", u64::MAX)
                .expect("complete improving chronological holdout should promote");
        assert_eq!(evidence.sample_count, 40);
        assert_eq!(evidence.training_count, 30);
        assert_eq!(evidence.validation_count, 10);
        assert!(evidence.validation_gain_percent >= MIN_VALIDATION_GAIN_PERCENT);
        assert_eq!(evidence.as_of_ms, 1_000_000 + 39 * 10_000_000 + 30 * 60_000);

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
        let report = report_replay(&samples[30..], evidence.parameters);
        assert!(report.selected_mean_net_percent > report.baseline_mean_net_percent);
        assert_eq!(report.actual_exit_reason_counts, [6, 2, 2]);
        assert!(report.hold_improvement_mean_percent > 0.0);
        assert!(report.hold_improvement_median_percent > 0.0);
        assert_eq!(report.hold_improvement_positive_rate, 0.6);
        assert_eq!(report.baseline_replay_exit_reason_counts[1..3], [6, 2]);
        assert_eq!(report.selected_replay_exit_reason_counts, [2, 0, 2, 6]);
        assert!(report.selected_max_drawdown_percent <= report.baseline_max_drawdown_percent);
        for direction in [&report.long_holdout, &report.short_holdout] {
            assert_eq!(direction.sample_count, 5);
            assert!(direction.selected_mean_net_percent > direction.baseline_mean_net_percent);
            assert!(direction.hold_improvement_mean_percent > 0.0);
            assert!(direction.hold_improvement_median_percent >= 0.0);
            assert_eq!(direction.hold_improvement_positive_rate, 0.6);
            assert_eq!(direction.actual_exit_reason_counts, [3, 1, 1]);
        }
    }

    #[test]
    fn chronological_replay_rejects_candidate_when_holdout_reverses_or_is_flat() {
        for case in [PathCase::HoldoutReverses, PathCase::HoldoutFlat] {
            let observations = synthetic_observations(case);
            assert!(
                select_exit_policy(observations.iter(), "BTC-USDT", "live-momentum", u64::MAX,)
                    .is_none(),
                "a worse or inconclusive later holdout must not promote"
            );
        }
    }

    #[test]
    fn chronological_replay_requires_adequate_post_exit_path_coverage() {
        let mut observations = synthetic_observations(PathCase::TrainingAndHoldoutImprove);
        for value in &mut observations {
            value["price_path"] =
                serde_json::Value::Array(value["price_path"].as_array().unwrap()[..8].to_vec());
        }
        assert!(
            select_exit_policy(observations.iter(), "BTC-USDT", "live-momentum", u64::MAX)
                .is_none()
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
}
