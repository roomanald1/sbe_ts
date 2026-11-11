use std::collections::VecDeque;

const VOLATILITY_HISTORY_MS: u64 = 300_000;

#[derive(Default)]
pub(crate) struct RollingVolatility {
    samples: VecDeque<(u64, f64)>,
    minute_returns: VecDeque<(u64, f64)>,
    window_returns: VecDeque<(u64, f64)>,
    minute_squared_sum: f64,
    window_squared_sum: f64,
}

impl RollingVolatility {
    pub(crate) fn update(&mut self, timestamp_ms: u64, price: f64) -> (f64, f64) {
        if let Some((last_timestamp, last_price)) = self.samples.back().copied() {
            if timestamp_ms < last_timestamp {
                return self.values();
            }
            if timestamp_ms == last_timestamp {
                if price.is_finite() && price > 0.0 && last_price.is_finite() && last_price > 0.0 {
                    let squared = (price / last_price).ln().powi(2);
                    self.replace_latest_return(timestamp_ms, squared);
                }
                return self.values();
            }
        }
        if price.is_finite() && price > 0.0 {
            if let Some((_, previous)) = self.samples.back().copied() {
                if previous.is_finite() && previous > 0.0 {
                    let squared = (price / previous).ln().powi(2);
                    self.prune_returns(timestamp_ms);
                    self.minute_returns.push_back((timestamp_ms, squared));
                    self.window_returns.push_back((timestamp_ms, squared));
                    self.minute_squared_sum += squared;
                    self.window_squared_sum += squared;
                }
            }
            self.samples.push_back((timestamp_ms, price));
            let oldest_to_keep = timestamp_ms.saturating_sub(VOLATILITY_HISTORY_MS);
            while self
                .samples
                .front()
                .is_some_and(|(time, _)| *time < oldest_to_keep)
            {
                self.samples.pop_front();
            }
            self.prune_returns(timestamp_ms);
        }
        self.values()
    }

    fn prune_returns(&mut self, timestamp_ms: u64) {
        let minute_cutoff = timestamp_ms.saturating_sub(60_000);
        while self
            .minute_returns
            .front()
            .is_some_and(|(time, _)| *time < minute_cutoff)
        {
            if let Some((_, value)) = self.minute_returns.pop_front() {
                self.minute_squared_sum -= value;
            }
        }
        let window_cutoff = timestamp_ms.saturating_sub(VOLATILITY_HISTORY_MS);
        while self
            .window_returns
            .front()
            .is_some_and(|(time, _)| *time < window_cutoff)
        {
            if let Some((_, value)) = self.window_returns.pop_front() {
                self.window_squared_sum -= value;
            }
        }
        self.minute_squared_sum = self.minute_squared_sum.max(0.0);
        self.window_squared_sum = self.window_squared_sum.max(0.0);
    }

    fn replace_latest_return(&mut self, timestamp_ms: u64, squared: f64) {
        if let Some((time, old)) = self.minute_returns.back_mut() {
            if *time == timestamp_ms {
                self.minute_squared_sum += squared - *old;
                *old = squared;
            }
        }
        if let Some((time, old)) = self.window_returns.back_mut() {
            if *time == timestamp_ms {
                self.window_squared_sum += squared - *old;
                *old = squared;
            }
        }
    }

    fn values(&self) -> (f64, f64) {
        (
            volatility_from_sum(self.minute_squared_sum, self.minute_returns.len()),
            volatility_from_sum(self.window_squared_sum, self.window_returns.len()),
        )
    }
}

fn volatility_from_sum(squared_returns: f64, return_count: usize) -> f64 {
    if return_count == 0 {
        f64::NAN
    } else {
        squared_returns.max(0.0).sqrt() * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::RollingVolatility;

    #[test]
    fn calculates_one_and_five_minute_realized_volatility() {
        let mut tracker = RollingVolatility::default();
        assert!(tracker.update(1_000, 100.0).0.is_nan());
        let (one_minute, five_minute) = tracker.update(2_000, 110.0);
        let expected = 1.1_f64.ln() * 100.0;
        assert!((one_minute - expected).abs() < 1e-12);
        assert_eq!(five_minute, one_minute);
        tracker.prune_returns(63_000);
        let aged = tracker.values();
        assert!(aged.0.is_nan());
        assert!(aged.1.is_finite());
    }

    #[test]
    fn replaces_same_timestamp_and_ignores_stale_prices() {
        let mut tracker = RollingVolatility::default();
        tracker.update(1_000, 100.0);
        tracker.update(2_000, 110.0);
        let first = tracker.update(2_000, 110.0);
        let updated = tracker.update(2_000, 120.0);
        assert_ne!(updated.0, first.0);
        let stale = tracker.update(1_500, 90.0);
        assert_eq!(stale, updated);
    }

    #[test]
    fn rolling_windows_keep_the_cutoff_timestamp_and_prune_older_returns() {
        let mut tracker = RollingVolatility::default();
        tracker.update(1_000, 100.0);
        tracker.update(2_000, 110.0);

        tracker.prune_returns(62_000);
        assert_eq!(tracker.minute_returns.len(), 1);
        assert_eq!(tracker.window_returns.len(), 1);

        tracker.prune_returns(62_001);
        assert!(tracker.minute_returns.is_empty());
        assert_eq!(tracker.window_returns.len(), 1);

        tracker.prune_returns(302_000);
        assert_eq!(tracker.window_returns.len(), 1);
        tracker.prune_returns(302_001);
        assert!(tracker.window_returns.is_empty());
        let (minute, five_minute) = tracker.values();
        assert!(minute.is_nan());
        assert!(five_minute.is_nan());
    }
}
