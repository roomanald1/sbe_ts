impl SignalEngine {
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
        format!(
            "price_samples={} latest_price_timestamp={:?} trend_1m={} momentum_10s={} recent_trades_60s={} book_imbalance={:?} quote_age_ms={quote_age:?}",
            self.prices.len(),
            latest.map(|sample| sample.0),
            self.price_return(timestamp.saturating_sub(60_000)).is_some(),
            self.price_return(timestamp.saturating_sub(10_000)).is_some(),
            recent_trades,
            self.book_imbalance,
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
