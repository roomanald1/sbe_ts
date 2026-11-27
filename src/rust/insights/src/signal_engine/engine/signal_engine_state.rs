impl SignalEngine {
    /// Keep momentum history advancing from both ticker updates and trades.
    /// Ticker and trade streams can arrive out of timestamp order, so insert
    /// each one-second sample into the time-ordered history instead of
    /// discarding older trade marks.
    fn record_price_sample(&mut self, timestamp: u64, price: f64) -> bool {
        if !price.is_finite() || price <= 0.0 {
            return false;
        }
        let bucket = timestamp / SIGNAL_PRICE_SAMPLE_INTERVAL_MS;
        let index = self
            .prices
            .partition_point(|sample| sample.0 / SIGNAL_PRICE_SAMPLE_INTERVAL_MS < bucket);
        if let Some(existing) = self
            .prices
            .get_mut(index)
            .filter(|sample| sample.0 / SIGNAL_PRICE_SAMPLE_INTERVAL_MS == bucket)
        {
            if timestamp >= existing.0 {
                *existing = (timestamp, price);
            }
        } else {
            self.prices.insert(index, (timestamp, price));
        }
        true
    }

    pub fn price_state(&self) -> (usize, Option<(u64, f64)>) {
        (self.prices.len(), self.prices.back().copied())
    }

    pub fn input_diagnostic(&self, timestamp: u64) -> String {
        let latest = self.prices.back().copied();
        let recent_trades = self
            .trades
            .iter()
            .filter(|(trade_time, ..)| {
                *trade_time >= timestamp.saturating_sub(TRADE_WINDOW_MS) && *trade_time <= timestamp
            })
            .count();
        let quote_age = self
            .best_quote
            .map(|(quote_time, _, _)| timestamp.saturating_sub(quote_time));
        let provenance = &self.book_input_provenance;
        let last_book_source_receive_delta_ms = provenance
            .last_received_at_ms
            .zip(provenance.last_source_timestamp_ms)
            .map(|(received_at, source_timestamp)| {
                i128::from(received_at) - i128::from(source_timestamp)
            });
        format!(
            "price_samples={} latest_price_timestamp={:?} trend_1m={} momentum_10s={} recent_trades_60s={} book_imbalance={:?} quote_age_ms={quote_age:?} book_events_received={} book_quotes_accepted={} book_older_rejected={} book_equal_rejected={} book_invalid_rejected={} book_last_source_timestamp_ms={:?} book_last_received_at_ms={:?} book_last_source_receive_delta_ms={:?} book_last_accepted_source_timestamp_ms={:?} book_last_accepted_at_ms={:?}",
            self.prices.len(),
            latest.map(|sample| sample.0),
            self.price_return(timestamp.saturating_sub(60_000))
                .is_some(),
            self.price_return(timestamp.saturating_sub(10_000))
                .is_some(),
            recent_trades,
            self.book_imbalance,
            provenance.received,
            provenance.accepted,
            provenance.older_timestamp_rejected,
            provenance.equal_timestamp_rejected,
            provenance.invalid_book_rejected,
            provenance.last_source_timestamp_ms,
            provenance.last_received_at_ms,
            last_book_source_receive_delta_ms,
            provenance.last_accepted_source_timestamp_ms,
            provenance.last_accepted_at_ms,
        )
    }

    fn realized_volatility(&self, cutoff: u64) -> Option<f64> {
        let mut previous: Option<f64> = None;
        let mut count = 0usize;
        let mut sum = 0.0f64;
        let start = self.prices.partition_point(|sample| sample.0 < cutoff);
        for &(_, price) in self.prices.range(start..) {
            count += 1;
            if let Some(previous_price) = previous {
                let change = (price / previous_price).ln();
                sum += change * change;
            }
            previous = Some(price);
        }
        (count >= 2).then_some(sum.sqrt() * 100.0)
    }

    fn price_return(&self, cutoff: u64) -> Option<f64> {
        let latest = self.prices.back()?.1;
        let index = self.prices.partition_point(|sample| sample.0 <= cutoff);
        if index == 0 {
            return None;
        }
        let (sample_time, first) = self.prices[index - 1];
        if cutoff.saturating_sub(sample_time) > MAX_PRICE_LOOKBACK_GAP_MS {
            return None;
        }
        Some((latest / first - 1.0) * 100.0)
    }

    fn prune(&mut self, now: u64) {
        while self
            .prices
            .front()
            .is_some_and(|sample| sample.0 < now.saturating_sub(PRICE_WINDOW_MS))
        {
            self.prices.pop_front();
        }
        while self
            .trades
            .front()
            .is_some_and(|sample| sample.0 < now.saturating_sub(TRADE_WINDOW_MS))
        {
            self.trades.pop_front();
        }
        while self
            .absorption_trades
            .front()
            .is_some_and(|sample| sample.timestamp < now.saturating_sub(ABSORPTION_WINDOW_MS))
        {
            self.absorption_trades.pop_front();
        }
    }
}
