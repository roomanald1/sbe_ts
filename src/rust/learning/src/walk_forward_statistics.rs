//! Dependence-aware summaries for completed Scout outcomes.

use serde::Serialize;
use std::collections::BTreeMap;

const ONLINE_WINDOW_MS: u64 = 4 * 60 * 60 * 1_000;
pub(super) const BOOTSTRAP_REPLICATES: usize = 512;
pub(super) const MEAN_BLOCK_LENGTH_DAYS: f64 = 7.0;
// Preserve roughly one day of serial dependence across six four-hour blocks.
const MEAN_BLOCK_LENGTH_ONLINE_WINDOWS: f64 = 6.0;
pub(super) const MINIMUM_SPA_DAY_CLUSTERS: usize = 30;

/// A complete daily portfolio row. Flat periods must be present as explicit
/// zero returns; callers must not construct this stream from trade-only rows.
#[derive(Clone, Debug)]
pub(super) struct DailyPortfolioReturns {
    pub utc_day: u64,
    pub baseline_return_percent: f64,
    pub candidate_returns_percent: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OosCandidateInference {
    pub status: String,
    pub candidate_id: String,
    pub day_clusters: usize,
    pub effective_sample_size: f64,
    pub mean_net_delta_percent: Option<f64>,
    pub expectancy_lcb_95_percent: Option<f64>,
    pub expectancy_ucb_95_percent: Option<f64>,
    pub max_drawdown_percent: Option<f64>,
    pub downside_loss_percent: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HansenSpaResult {
    pub status: String,
    pub day_clusters: usize,
    pub candidate_count: usize,
    pub test_statistic: Option<f64>,
    pub p_value: Option<f64>,
    pub best_mean_delta_candidate_id: Option<String>,
    pub best_mean_delta_percent: Option<f64>,
    pub bootstrap_replicates: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct WalkForwardSample {
    pub observed_at_ms: u64,
    pub net_return_percent: f64,
    pub weight: f64,
}

#[derive(Default)]
pub(super) struct WalkForwardStatistics {
    pub sample_count: usize,
    pub positive_net_rate: f64,
    pub average_net_return_percent: f64,
    pub expectancy_ci_low_percent: Option<f64>,
    pub expectancy_ci_high_percent: Option<f64>,
    /// Effective number of four-hour outcome windows, not raw trades.
    pub effective_sample_size: f64,
    pub window_count: usize,
    pub max_drawdown_percent: Option<f64>,
    pub downside_loss_percent: Option<f64>,
}

pub(super) fn summarize_walk_forward_samples(
    mut samples: Vec<WalkForwardSample>,
) -> WalkForwardStatistics {
    samples.retain(|sample| {
        sample.net_return_percent.is_finite() && sample.weight.is_finite() && sample.weight > 0.0
    });
    samples.sort_by_key(|sample| sample.observed_at_ms);
    if samples.is_empty() {
        return WalkForwardStatistics::default();
    }

    // Update live evidence several times per day while keeping bursts of
    // tightly spaced, related trades inside the same dependence cluster.
    let mut windows = Vec::<(u64, f64, f64, f64)>::new();
    for sample in &samples {
        let window = sample.observed_at_ms / ONLINE_WINDOW_MS;
        if let Some(last) = windows.last_mut().filter(|row| row.0 == window) {
            last.1 += sample.net_return_percent * sample.weight;
            last.2 += sample.weight;
            last.3 += sample.weight * f64::from(sample.net_return_percent > 0.0);
        } else {
            windows.push((
                window,
                sample.net_return_percent * sample.weight,
                sample.weight,
                sample.weight * f64::from(sample.net_return_percent > 0.0),
            ));
        }
    }

    let weight_sum = windows.iter().map(|window| window.2).sum::<f64>();
    let squared_weight_sum = windows
        .iter()
        .map(|window| window.2 * window.2)
        .sum::<f64>();
    let mean = windows.iter().map(|window| window.1).sum::<f64>() / weight_sum;
    let positive_weight = windows.iter().map(|window| window.3).sum::<f64>();
    let mut equity = 1.0_f64;
    let mut peak = 1.0_f64;
    let mut max_drawdown: f64 = 0.0;
    let mut loss_weight = 0.0;
    let mut loss_total = 0.0;
    for sample in &samples {
        equity *= (1.0 + sample.net_return_percent / 100.0).max(0.0);
        peak = peak.max(equity);
        max_drawdown = max_drawdown.max((peak - equity) / peak * 100.0);
        if sample.net_return_percent < 0.0 {
            loss_weight += sample.weight;
            loss_total += -sample.net_return_percent * sample.weight;
        }
    }

    let (ci_low, ci_high) = stationary_bootstrap_interval(&windows);
    WalkForwardStatistics {
        sample_count: samples.len(),
        positive_net_rate: positive_weight / weight_sum,
        average_net_return_percent: mean,
        expectancy_ci_low_percent: ci_low,
        expectancy_ci_high_percent: ci_high,
        effective_sample_size: weight_sum * weight_sum / squared_weight_sum,
        window_count: windows.len(),
        max_drawdown_percent: Some(max_drawdown),
        downside_loss_percent: (loss_weight > 0.0).then_some(loss_total / loss_weight),
    }
}

/// Estimate selected-candidate daily net expectancy against baseline. This
/// accepts only aligned, complete daily portfolio streams, including explicit
/// cash returns for days with no position.
pub(super) fn summarize_oos_candidate(
    rows: &[DailyPortfolioReturns],
    candidate_id: &str,
    source_history_complete: bool,
    cash_periods_established: bool,
    bootstrap_replicates: usize,
    expected_block_length_days: f64,
) -> OosCandidateInference {
    let mut result = OosCandidateInference {
        status: "incomplete".into(),
        candidate_id: candidate_id.to_owned(),
        day_clusters: rows.len(),
        effective_sample_size: 0.0,
        mean_net_delta_percent: None,
        expectancy_lcb_95_percent: None,
        expectancy_ucb_95_percent: None,
        max_drawdown_percent: None,
        downside_loss_percent: None,
    };
    if !source_history_complete {
        result.status = "source_history_incomplete".into();
        return result;
    }
    if !cash_periods_established {
        result.status = "cash_periods_unestablished".into();
        return result;
    }
    if bootstrap_replicates == 0
        || !expected_block_length_days.is_finite()
        || expected_block_length_days <= 0.0
    {
        result.status = "invalid_inference_configuration".into();
        return result;
    }
    if let Err(status) = validate_daily_portfolio_rows(rows) {
        result.status = status.into();
        return result;
    }
    let Some(first) = rows.first() else {
        result.status = "no_daily_portfolio_rows".into();
        return result;
    };
    if !first.candidate_returns_percent.contains_key(candidate_id) {
        result.status = "candidate_stream_missing".into();
        return result;
    }
    let candidate_returns = rows
        .iter()
        .map(|row| row.candidate_returns_percent[candidate_id])
        .collect::<Vec<_>>();
    let deltas = rows
        .iter()
        .map(|row| row.candidate_returns_percent[candidate_id] - row.baseline_return_percent)
        .collect::<Vec<_>>();
    result.effective_sample_size = rows.len() as f64;
    result.mean_net_delta_percent = Some(mean(&deltas));
    let mut seed = daily_rows_seed(rows);
    let mut bootstrapped_means = stationary_bootstrap_means(
        &deltas,
        &mut seed,
        bootstrap_replicates,
        expected_block_length_days,
    );
    bootstrapped_means.sort_by(f64::total_cmp);
    if bootstrapped_means.len() == BOOTSTRAP_REPLICATES {
        result.expectancy_lcb_95_percent =
            Some(bootstrapped_means[percentile_index(0.05, bootstrapped_means.len())]);
        result.expectancy_ucb_95_percent =
            Some(bootstrapped_means[percentile_index(0.95, bootstrapped_means.len())]);
    }
    let mut equity = 1.0_f64;
    let mut peak = 1.0_f64;
    let mut max_drawdown: f64 = 0.0;
    let mut losses = Vec::new();
    for value in candidate_returns {
        equity *= (1.0 + value / 100.0).max(0.0);
        peak = peak.max(equity);
        max_drawdown = max_drawdown.max((peak - equity) / peak * 100.0);
        if value < 0.0 {
            losses.push(-value);
        }
    }
    result.max_drawdown_percent = Some(max_drawdown);
    result.downside_loss_percent = (!losses.is_empty()).then(|| mean(&losses));
    result.status = if rows.len() < 3 {
        "insufficient_day_clusters".into()
    } else {
        "available".into()
    };
    result
}

/// Hansen's studentized SPA test with the consistent, sample-dependent null
/// centering and a stationary bootstrap over complete UTC-day portfolio rows.
/// It is deliberately unavailable unless the caller establishes full source
/// coverage and the complete cash/held timeline.
pub(super) fn stationary_bootstrap_spa(
    rows: &[DailyPortfolioReturns],
    source_history_complete: bool,
    cash_periods_established: bool,
    bootstrap_replicates: usize,
    expected_block_length_days: f64,
    minimum_day_clusters: usize,
) -> HansenSpaResult {
    let mut result = HansenSpaResult {
        status: "incomplete".into(),
        day_clusters: rows.len(),
        candidate_count: rows
            .first()
            .map_or(0, |row| row.candidate_returns_percent.len()),
        test_statistic: None,
        p_value: None,
        best_mean_delta_candidate_id: None,
        best_mean_delta_percent: None,
        bootstrap_replicates,
    };
    if !source_history_complete {
        result.status = "source_history_incomplete".into();
        return result;
    }
    if !cash_periods_established {
        result.status = "cash_periods_unestablished".into();
        return result;
    }
    if bootstrap_replicates == 0
        || !expected_block_length_days.is_finite()
        || expected_block_length_days <= 0.0
        || minimum_day_clusters == 0
    {
        result.status = "invalid_inference_configuration".into();
        return result;
    }
    if let Err(status) = validate_daily_portfolio_rows(rows) {
        result.status = status.into();
        return result;
    }
    if rows.len() < minimum_day_clusters {
        result.status = "insufficient_day_clusters".into();
        return result;
    }
    let Some(first) = rows.first() else {
        result.status = "no_daily_portfolio_rows".into();
        return result;
    };
    let candidate_ids = first
        .candidate_returns_percent
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    if candidate_ids.is_empty() {
        result.status = "searched_candidate_family_missing".into();
        return result;
    }
    let day_count = rows.len();
    let candidate_deltas = candidate_ids
        .iter()
        .map(|candidate_id| {
            rows.iter()
                .map(|row| {
                    row.candidate_returns_percent[candidate_id] - row.baseline_return_percent
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let candidate_means = candidate_deltas
        .iter()
        .map(|values| mean(values))
        .collect::<Vec<_>>();
    let long_run_sd = candidate_deltas
        .iter()
        .map(|values| stationary_long_run_sd(values, expected_block_length_days))
        .collect::<Vec<_>>();
    if long_run_sd
        .iter()
        .any(|sd| !sd.is_finite() || *sd <= f64::EPSILON)
    {
        result.status = "degenerate_long_run_variance".into();
        return result;
    }
    let best_index = candidate_means
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(index, _)| index)
        .expect("candidate family was checked nonempty");
    let observed_statistic = candidate_means
        .iter()
        .zip(&long_run_sd)
        .map(|(candidate_mean, sd)| (day_count as f64).sqrt() * candidate_mean / sd)
        .fold(0.0_f64, f64::max);
    result.test_statistic = Some(observed_statistic);
    result.best_mean_delta_candidate_id = Some(candidate_ids[best_index].clone());
    result.best_mean_delta_percent = Some(candidate_means[best_index]);

    // Hansen's consistent null estimate keeps only alternatives that are not
    // clearly inferior according to the sqrt(2 log log n) threshold.
    let threshold = (2.0 * (day_count as f64).ln().ln()).sqrt();
    let null_means = candidate_means
        .iter()
        .zip(&long_run_sd)
        .map(|(candidate_mean, sd)| {
            if (day_count as f64).sqrt() * candidate_mean / sd >= -threshold {
                *candidate_mean
            } else {
                0.0
            }
        })
        .collect::<Vec<_>>();

    let mut seed = daily_rows_seed(rows);
    let mut exceedances = 0usize;
    for _ in 0..bootstrap_replicates {
        let indices =
            stationary_bootstrap_indices(day_count, &mut seed, expected_block_length_days);
        let bootstrap_statistic = candidate_deltas
            .iter()
            .zip(&long_run_sd)
            .zip(&null_means)
            .map(|((values, sd), null_mean)| {
                let centered_mean = indices
                    .iter()
                    .map(|index| values[*index] - null_mean)
                    .sum::<f64>()
                    / day_count as f64;
                (day_count as f64).sqrt() * centered_mean / sd
            })
            .fold(0.0_f64, f64::max);
        if bootstrap_statistic >= observed_statistic {
            exceedances += 1;
        }
    }
    result.p_value = Some((exceedances + 1) as f64 / (bootstrap_replicates + 1) as f64);
    result.status = "available".into();
    result
}

fn validate_daily_portfolio_rows(rows: &[DailyPortfolioReturns]) -> Result<(), &'static str> {
    let Some(first) = rows.first() else {
        return Err("no_daily_portfolio_rows");
    };
    let candidate_ids = first.candidate_returns_percent.keys().collect::<Vec<_>>();
    if rows.iter().any(|row| {
        !row.baseline_return_percent.is_finite()
            || row.candidate_returns_percent.keys().collect::<Vec<_>>() != candidate_ids
            || row
                .candidate_returns_percent
                .values()
                .any(|value| !value.is_finite())
    }) {
        return Err("candidate_streams_not_aligned_or_non_finite");
    }
    if rows
        .windows(2)
        .any(|pair| pair[1].utc_day != pair[0].utc_day.saturating_add(1))
    {
        return Err("daily_portfolio_stream_has_unestablished_gaps");
    }
    Ok(())
}

fn stationary_bootstrap_means(
    values: &[f64],
    seed: &mut u64,
    bootstrap_replicates: usize,
    expected_block_length_days: f64,
) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }
    (0..bootstrap_replicates)
        .map(|_| {
            let indices =
                stationary_bootstrap_indices(values.len(), seed, expected_block_length_days);
            indices.iter().map(|index| values[*index]).sum::<f64>() / values.len() as f64
        })
        .collect()
}

fn stationary_bootstrap_indices(
    length: usize,
    seed: &mut u64,
    expected_block_length_days: f64,
) -> Vec<usize> {
    let restart_probability = 1.0 / expected_block_length_days;
    let mut indices = Vec::with_capacity(length);
    let mut current = (splitmix64(*seed) as usize) % length;
    *seed = splitmix64(*seed);
    for index in 0..length {
        if index > 0 && (splitmix64(*seed) as f64 / u64::MAX as f64) < restart_probability {
            current = (splitmix64(*seed) as usize) % length;
        } else if index > 0 {
            current = (current + 1) % length;
        }
        *seed = splitmix64(*seed);
        indices.push(current);
    }
    indices
}

fn stationary_long_run_sd(values: &[f64], expected_block_length_days: f64) -> f64 {
    let length = values.len();
    if length < 2 {
        return f64::NAN;
    }
    let center = mean(values);
    let centered = values
        .iter()
        .map(|value| value - center)
        .collect::<Vec<_>>();
    let mut variance = centered.iter().map(|value| value * value).sum::<f64>() / length as f64;
    let restart_probability = 1.0 / expected_block_length_days;
    for lag in 1..length {
        let autocovariance = centered[..length - lag]
            .iter()
            .zip(&centered[lag..])
            .map(|(left, right)| left * right)
            .sum::<f64>()
            / length as f64;
        let kernel = (length - lag) as f64 / length as f64
            * (1.0 - restart_probability).powi(lag as i32)
            + lag as f64 / length as f64 * (1.0 - restart_probability).powi((length - lag) as i32);
        variance += 2.0 * kernel * autocovariance;
    }
    variance.max(0.0).sqrt()
}

fn daily_rows_seed(rows: &[DailyPortfolioReturns]) -> u64 {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for row in rows {
        seed ^= row.utc_day.rotate_left(13) ^ row.baseline_return_percent.to_bits().rotate_left(29);
        for (candidate_id, value) in &row.candidate_returns_percent {
            for byte in candidate_id.as_bytes() {
                seed = splitmix64(seed ^ u64::from(*byte));
            }
            seed = splitmix64(seed ^ value.to_bits().rotate_left(47));
        }
    }
    seed
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn percentile_index(percentile: f64, length: usize) -> usize {
    ((percentile * length as f64).floor() as usize).min(length.saturating_sub(1))
}

fn stationary_bootstrap_interval(windows: &[(u64, f64, f64, f64)]) -> (Option<f64>, Option<f64>) {
    if windows.len() < 3 {
        return (None, None);
    }
    let restart_probability = 1.0 / MEAN_BLOCK_LENGTH_ONLINE_WINDOWS;
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for (window, weighted_return, weight, _) in windows {
        seed ^= window.rotate_left(13)
            ^ weighted_return.to_bits().rotate_left(29)
            ^ weight.to_bits().rotate_left(47);
        seed = splitmix64(seed);
    }

    let mut estimates = Vec::with_capacity(BOOTSTRAP_REPLICATES);
    let length = windows.len();
    for _ in 0..BOOTSTRAP_REPLICATES {
        let mut index = (splitmix64(seed) as usize) % length;
        seed = splitmix64(seed);
        let (mut weighted_return, mut weight) = (0.0, 0.0);
        for _ in 0..length {
            let window = windows[index];
            weighted_return += window.1;
            weight += window.2;
            let restart = (splitmix64(seed) as f64 / u64::MAX as f64) < restart_probability;
            seed = splitmix64(seed);
            let next = (index + 1) % length;
            let has_calendar_gap = windows[next].0 > window.0.saturating_add(1);
            index = if restart || has_calendar_gap {
                (splitmix64(seed) as usize) % length
            } else {
                next
            };
        }
        estimates.push(weighted_return / weight);
    }
    estimates.sort_by(f64::total_cmp);
    let low =
        ((BOOTSTRAP_REPLICATES as f64 * 0.025).floor() as usize).min(BOOTSTRAP_REPLICATES - 1);
    let high =
        ((BOOTSTRAP_REPLICATES as f64 * 0.975).ceil() as usize - 1).min(BOOTSTRAP_REPLICATES - 1);
    (Some(estimates[low]), Some(estimates[high]))
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(window: u64, value: f64) -> WalkForwardSample {
        WalkForwardSample {
            observed_at_ms: window * ONLINE_WINDOW_MS + 1_000,
            net_return_percent: value,
            weight: 1.0,
        }
    }

    #[test]
    fn stationary_bootstrap_is_deterministic_and_uses_chronological_four_hour_blocks() {
        let positive = (0..30)
            .map(|window| sample(window, 0.2))
            .collect::<Vec<_>>();
        let first = summarize_walk_forward_samples(positive.clone());
        let second = summarize_walk_forward_samples(positive);

        assert_eq!(
            first.expectancy_ci_low_percent,
            second.expectancy_ci_low_percent
        );
        assert_eq!(
            first.expectancy_ci_high_percent,
            second.expectancy_ci_high_percent
        );
        assert!(first.expectancy_ci_low_percent.unwrap() > 0.0);
        assert_eq!(first.effective_sample_size, 30.0);
        assert_eq!(first.window_count, 30);
    }

    #[test]
    fn outcomes_in_one_four_hour_window_form_one_effective_sample_cluster() {
        let samples = vec![
            sample(0, 0.2),
            sample(0, 0.4),
            sample(1, 0.3),
            sample(2, 0.1),
        ];
        let result = summarize_walk_forward_samples(samples);
        assert_eq!(result.sample_count, 4);
        assert_eq!(result.window_count, 3);
        assert!((result.effective_sample_size - 8.0 / 3.0).abs() < 1e-12);
        assert!(result.expectancy_ci_low_percent.is_some());
    }

    #[test]
    fn interval_is_unavailable_without_three_independent_four_hour_windows() {
        let result = summarize_walk_forward_samples(vec![sample(10, 0.2), sample(10, 0.3)]);
        assert_eq!(result.expectancy_ci_low_percent, None);
        assert_eq!(result.expectancy_ci_high_percent, None);
        assert_eq!(result.effective_sample_size, 1.0);
    }

    fn complete_portfolio_rows(days: usize) -> Vec<DailyPortfolioReturns> {
        (0..days)
            .map(|day| {
                let variation = match day % 3 {
                    0 => -0.04,
                    1 => 0.0,
                    _ => 0.04,
                };
                DailyPortfolioReturns {
                    utc_day: day as u64,
                    baseline_return_percent: 0.0,
                    candidate_returns_percent: BTreeMap::from([
                        ("better".into(), 0.3 + variation),
                        ("worse".into(), -0.2 + variation),
                    ]),
                }
            })
            .collect()
    }

    #[test]
    fn oos_expectancy_and_spa_are_deterministic_on_aligned_complete_portfolio_days() {
        let rows = complete_portfolio_rows(60);
        let first = summarize_oos_candidate(
            &rows,
            "better",
            true,
            true,
            BOOTSTRAP_REPLICATES,
            MEAN_BLOCK_LENGTH_DAYS,
        );
        let second = summarize_oos_candidate(
            &rows,
            "better",
            true,
            true,
            BOOTSTRAP_REPLICATES,
            MEAN_BLOCK_LENGTH_DAYS,
        );
        assert_eq!(first.status, "available");
        assert!(
            first
                .mean_net_delta_percent
                .is_some_and(|mean| (mean - 0.3).abs() < 1e-12)
        );
        assert!(first.expectancy_lcb_95_percent.unwrap() > 0.0);
        assert_eq!(
            first.expectancy_lcb_95_percent,
            second.expectancy_lcb_95_percent
        );

        let spa_first = stationary_bootstrap_spa(
            &rows,
            true,
            true,
            BOOTSTRAP_REPLICATES,
            MEAN_BLOCK_LENGTH_DAYS,
            MINIMUM_SPA_DAY_CLUSTERS,
        );
        let spa_second = stationary_bootstrap_spa(
            &rows,
            true,
            true,
            BOOTSTRAP_REPLICATES,
            MEAN_BLOCK_LENGTH_DAYS,
            MINIMUM_SPA_DAY_CLUSTERS,
        );
        assert_eq!(spa_first.status, "available");
        assert_eq!(
            spa_first.best_mean_delta_candidate_id.as_deref(),
            Some("better")
        );
        assert!(spa_first.p_value.unwrap() < 0.05);
        assert_eq!(spa_first.p_value, spa_second.p_value);
    }

    #[test]
    fn oos_inference_fails_closed_for_unestablished_cash_and_gapped_days() {
        let mut rows = complete_portfolio_rows(30);
        assert_eq!(
            summarize_oos_candidate(
                &rows,
                "better",
                true,
                false,
                BOOTSTRAP_REPLICATES,
                MEAN_BLOCK_LENGTH_DAYS,
            )
            .status,
            "cash_periods_unestablished"
        );
        rows[10].utc_day = 11;
        assert_eq!(
            stationary_bootstrap_spa(
                &rows,
                true,
                true,
                BOOTSTRAP_REPLICATES,
                MEAN_BLOCK_LENGTH_DAYS,
                MINIMUM_SPA_DAY_CLUSTERS,
            )
            .status,
            "daily_portfolio_stream_has_unestablished_gaps"
        );
    }
}
