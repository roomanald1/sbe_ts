//! Dependence-aware summaries for completed Scout outcomes.

const UTC_DAY_MS: u64 = 24 * 60 * 60 * 1_000;
const BOOTSTRAP_REPLICATES: usize = 512;
const MEAN_BLOCK_LENGTH_DAYS: f64 = 7.0;

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
    /// Effective number of outcome-day clusters, not raw trades.
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

    // Bootstrap daily clusters so multiple outcomes from the same UTC day do
    // not masquerade as independent observations.
    let mut days = Vec::<(u64, f64, f64, f64)>::new();
    for sample in &samples {
        let day = sample.observed_at_ms / UTC_DAY_MS;
        if let Some(last) = days.last_mut().filter(|row| row.0 == day) {
            last.1 += sample.net_return_percent * sample.weight;
            last.2 += sample.weight;
            last.3 += sample.weight * f64::from(sample.net_return_percent > 0.0);
        } else {
            days.push((
                day,
                sample.net_return_percent * sample.weight,
                sample.weight,
                sample.weight * f64::from(sample.net_return_percent > 0.0),
            ));
        }
    }

    let weight_sum = days.iter().map(|day| day.2).sum::<f64>();
    let squared_weight_sum = days.iter().map(|day| day.2 * day.2).sum::<f64>();
    let mean = days.iter().map(|day| day.1).sum::<f64>() / weight_sum;
    let positive_weight = days.iter().map(|day| day.3).sum::<f64>();
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

    let (ci_low, ci_high) = stationary_bootstrap_interval(&days);
    WalkForwardStatistics {
        sample_count: samples.len(),
        positive_net_rate: positive_weight / weight_sum,
        average_net_return_percent: mean,
        expectancy_ci_low_percent: ci_low,
        expectancy_ci_high_percent: ci_high,
        effective_sample_size: weight_sum * weight_sum / squared_weight_sum,
        window_count: days.len(),
        max_drawdown_percent: Some(max_drawdown),
        downside_loss_percent: (loss_weight > 0.0).then_some(loss_total / loss_weight),
    }
}

fn stationary_bootstrap_interval(days: &[(u64, f64, f64, f64)]) -> (Option<f64>, Option<f64>) {
    if days.len() < 3 {
        return (None, None);
    }
    let restart_probability = 1.0 / MEAN_BLOCK_LENGTH_DAYS;
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for (day, weighted_return, weight, _) in days {
        seed ^= day.rotate_left(13)
            ^ weighted_return.to_bits().rotate_left(29)
            ^ weight.to_bits().rotate_left(47);
        seed = splitmix64(seed);
    }

    let mut estimates = Vec::with_capacity(BOOTSTRAP_REPLICATES);
    let length = days.len();
    for _ in 0..BOOTSTRAP_REPLICATES {
        let mut index = (splitmix64(seed) as usize) % length;
        seed = splitmix64(seed);
        let (mut weighted_return, mut weight) = (0.0, 0.0);
        for _ in 0..length {
            let day = days[index];
            weighted_return += day.1;
            weight += day.2;
            let restart = (splitmix64(seed) as f64 / u64::MAX as f64) < restart_probability;
            seed = splitmix64(seed);
            let next = (index + 1) % length;
            let has_calendar_gap = days[next].0 > day.0.saturating_add(1);
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

    fn sample(day: u64, value: f64) -> WalkForwardSample {
        WalkForwardSample {
            observed_at_ms: day * UTC_DAY_MS + 1_000,
            net_return_percent: value,
            weight: 1.0,
        }
    }

    #[test]
    fn stationary_bootstrap_is_deterministic_and_uses_chronological_day_blocks() {
        let positive = (0..30).map(|day| sample(day, 0.2)).collect::<Vec<_>>();
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
    fn same_day_outcomes_form_one_effective_sample_cluster() {
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
    fn interval_is_unavailable_without_three_independent_day_clusters() {
        let result = summarize_walk_forward_samples(vec![sample(10, 0.2), sample(10, 0.3)]);
        assert_eq!(result.expectancy_ci_low_percent, None);
        assert_eq!(result.expectancy_ci_high_percent, None);
        assert_eq!(result.effective_sample_size, 1.0);
    }
}
