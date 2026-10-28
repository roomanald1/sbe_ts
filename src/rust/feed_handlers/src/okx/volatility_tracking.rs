impl RollingVolatility {
    pub(crate) fn update(&mut self, timestamp_ms: u64, price: f64) -> (f64, f64) {
        if let Some(&(last_timestamp, _)) = self.samples.back() {
            if timestamp_ms < last_timestamp {
                return self.values(timestamp_ms);
            }
            if timestamp_ms == last_timestamp {
                if let Some(last_sample) = self.samples.back_mut() {
                    last_sample.1 = price;
                }
                if self.samples.len() > 1 {
                    let previous = self.samples[self.samples.len() - 2].1;
                    let squared = (price / previous).ln().powi(2);
                    self.replace_latest_return(timestamp_ms, squared);
                }
                return self.values(timestamp_ms);
            }
        }

        let previous_price = self.samples.back().map(|sample| sample.1);
        self.samples.push_back((timestamp_ms, price));
        if let Some(previous_price) = previous_price {
            let squared = (price / previous_price).ln().powi(2);
            self.prune_returns(timestamp_ms);
            self.minute_returns.push_back((timestamp_ms, squared));
            self.window_returns.push_back((timestamp_ms, squared));
            self.minute_squared_sum += squared;
            self.window_squared_sum += squared;
        }

        let oldest_to_keep = timestamp_ms.saturating_sub(VOLATILITY_HISTORY_MS);
        while self.samples.len() > 2 && self.samples[1].0 < oldest_to_keep {
            self.samples.pop_front();
        }
        self.values(timestamp_ms)
    }

    fn prune_returns(&mut self, timestamp_ms: u64) {
        let minute_cutoff = timestamp_ms.saturating_sub(60_000);
        while self
            .minute_returns
            .front()
            .is_some_and(|(time, _)| *time <= minute_cutoff)
        {
            if let Some((_, squared)) = self.minute_returns.pop_front() {
                self.minute_squared_sum -= squared;
            }
        }
        let window_cutoff = timestamp_ms.saturating_sub(VOLATILITY_HISTORY_MS);
        while self
            .window_returns
            .front()
            .is_some_and(|(time, _)| *time <= window_cutoff)
        {
            if let Some((_, squared)) = self.window_returns.pop_front() {
                self.window_squared_sum -= squared;
            }
        }
    }

    fn replace_latest_return(&mut self, timestamp_ms: u64, squared: f64) {
        if let Some((time, previous)) = self.minute_returns.back_mut() {
            if *time == timestamp_ms {
                self.minute_squared_sum += squared - *previous;
                *previous = squared;
            }
        }
        if let Some((time, previous)) = self.window_returns.back_mut() {
            if *time == timestamp_ms {
                self.window_squared_sum += squared - *previous;
                *previous = squared;
            }
        }
    }

    fn values(&self, timestamp_ms: u64) -> (f64, f64) {
        if self
            .samples
            .back()
            .is_some_and(|(time, _)| *time == timestamp_ms)
        {
            return (
                volatility_from_sum(self.minute_squared_sum, self.minute_returns.len()),
                volatility_from_sum(self.window_squared_sum, self.window_returns.len()),
            );
        }
        (
            self.realized_volatility(timestamp_ms, 60_000),
            self.realized_volatility(timestamp_ms, VOLATILITY_HISTORY_MS),
        )
    }

    fn realized_volatility(&self, timestamp_ms: u64, window_ms: u64) -> f64 {
        let cutoff = timestamp_ms.saturating_sub(window_ms);
        let mut squared_returns = 0.0;
        let mut return_count = 0;
        let mut samples = self.samples.iter();
        let Some(mut previous) = samples.next() else {
            return f64::NAN;
        };
        for current in samples {
            if current.0 > cutoff {
                let log_return = (current.1 / previous.1).ln();
                squared_returns += log_return * log_return;
                return_count += 1;
            }
            previous = current;
        }
        volatility_from_sum(squared_returns, return_count)
    }
}

fn volatility_from_sum(squared_returns: f64, return_count: usize) -> f64 {
    if return_count == 0 {
        f64::NAN
    } else {
        squared_returns.max(0.0).sqrt() * 100.0
    }
}

