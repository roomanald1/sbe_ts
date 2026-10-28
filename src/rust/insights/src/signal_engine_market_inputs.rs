impl SignalEngine {
    pub fn book_snapshot(&self, symbol: &str, timestamp: u64) -> Option<SignalBookSnapshot> {
        let (quote_timestamp, _, _) = self.best_quote?;
        if quote_timestamp > timestamp
            || timestamp.saturating_sub(quote_timestamp) > MAX_BOOK_AGE_MS
        {
            return None;
        }
        Some(SignalBookSnapshot {
            symbol: symbol.to_owned(),
            timestamp,
            bid_levels: self.book_bid_levels.clone(),
            ask_levels: self.book_ask_levels.clone(),
        })
    }

    pub fn track(&mut self, snapshot: &SignalSnapshot) {
        if let Some((_, base_price)) = self.prices.back().copied() {
            let quote = self.best_quote.filter(|(quote_time, _, _)| {
                *quote_time <= snapshot.timestamp
                    && snapshot.timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
            });
            // Keep short-term checks while adding swing-style horizons. Outcomes are
            // evaluated from backend price/book data and persisted with the same cost model.
            for horizon_seconds in [60, 300, 900, 3_600] {
                self.pending.push_back(PendingOutcome {
                    symbol: snapshot.symbol.clone(),
                    timestamp: snapshot.timestamp,
                    base_price,
                    direction: snapshot.direction.clone(),
                    strength: snapshot.strength,
                    horizon_seconds,
                    entry_bid: quote.map(|(_, bid, _)| bid),
                    entry_ask: quote.map(|(_, _, ask)| ask),
                    trend_5m_volatility_units: snapshot.components.trend_5m_volatility_units,
                    features: feature_vector(&snapshot.components),
                    score: snapshot.score,
                    ranging_probability: snapshot.ranging_probability,
                    entry_quote_timestamp: quote.map(|(quote_time, _, _)| quote_time),
                    taker_fee_rate: configured_signal_taker_fee_rate(),
                    first_long_positive_net_timestamp: None,
                    first_short_positive_net_timestamp: None,
                });
            }
        }
    }

    pub fn on_trade(&mut self, timestamp: u64, price: f64, size: f64, is_buy: bool) {
        if !price.is_finite() || price <= 0.0 || !size.is_finite() || size <= 0.0 {
            return;
        }
        let aggressor_cost_bps = self
            .best_quote
            .filter(|(quote_time, bid, ask)| {
                *quote_time <= timestamp
                    && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                    && bid.is_finite()
                    && ask.is_finite()
                    && *bid > 0.0
                    && *ask >= *bid
            })
            .map(|(_, bid, ask)| {
                let midpoint = (bid + ask) / 2.0;
                let paid_vs_mid = if is_buy {
                    price - midpoint
                } else {
                    midpoint - price
                };
                paid_vs_mid / midpoint * 10_000.0
            })
            .filter(|cost| cost.is_finite());
        self.trades
            .push_back((timestamp, price, size, is_buy, aggressor_cost_bps));
        if let Some((_quote_time, bid, ask)) = self.best_quote.filter(|(qt, bid, ask)| {
            *qt <= timestamp
                && timestamp.saturating_sub(*qt) <= MAX_BOOK_AGE_MS
                && bid.is_finite()
                && ask.is_finite()
                && *bid > 0.0
                && *ask >= *bid
        }) {
            let depth_at = |levels: &[(f64, f64)], target: f64| {
                levels
                    .iter()
                    .find(|(level, _)| (*level - target).abs() <= target * 1e-8)
                    .map(|(_, quantity)| *quantity)
                    .unwrap_or(0.0)
            };
            self.absorption_trades.push_back(AbsorptionTrade {
                timestamp,
                price,
                size,
                is_buy,
                bid,
                ask,
                bid_depth: depth_at(&self.book_bid_levels, bid),
                ask_depth: depth_at(&self.book_ask_levels, ask),
            });
        }
        self.prune(timestamp);
    }

    pub fn on_book(
        &mut self,
        timestamp: u64,
        best_bid: f64,
        best_ask: f64,
        bid_sizes: &[f64],
        ask_sizes: &[f64],
        bid_levels: &[(f64, f64)],
        ask_levels: &[(f64, f64)],
    ) {
        if best_bid.is_finite() && best_ask.is_finite() && best_bid > 0.0 && best_ask >= best_bid {
            self.best_quote = Some((timestamp, best_bid, best_ask));
        }
        self.book_bid_levels.clear();
        self.book_bid_levels
            .extend(bid_levels.iter().copied().filter(|(price, size)| {
                price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0
            }));
        self.book_ask_levels.clear();
        self.book_ask_levels
            .extend(ask_levels.iter().copied().filter(|(price, size)| {
                price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0
            }));
        if best_bid > 0.0 && best_ask >= best_bid {
            let mut bid_levels = [(0.0, 0.0); 7];
            let mut ask_levels = [(0.0, 0.0); 7];
            for (target, source) in bid_levels.iter_mut().zip(self.book_bid_levels.iter()) {
                *target = *source;
            }
            for (target, source) in ask_levels.iter_mut().zip(self.book_ask_levels.iter()) {
                *target = *source;
            }
            self.book_history.push_back(BookHistorySample {
                timestamp,
                bid: best_bid,
                ask: best_ask,
                bid_levels,
                ask_levels,
            });
            while self
                .book_history
                .front()
                .is_some_and(|sample| sample.timestamp < timestamp.saturating_sub(60_000))
            {
                self.book_history.pop_front();
            }
        }
        let bid: f64 = bid_sizes
            .iter()
            .copied()
            .filter(|n| n.is_finite() && *n > 0.0)
            .sum();
        let ask: f64 = ask_sizes
            .iter()
            .copied()
            .filter(|n| n.is_finite() && *n > 0.0)
            .sum();
        if bid + ask > 0.0 {
            self.book_imbalance = Some((bid - ask) / (bid + ask));
        }
    }
}
