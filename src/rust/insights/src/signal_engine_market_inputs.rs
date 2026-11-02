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
                    sample_id: None,
                    strategy: None,
                    scout_parameter_key: None,
                    scout_parameter_value: None,
                    scout_minimum_hold_days: None,
                    scout_maximum_hold_days: None,
                    stop_loss_percent: None,
                    take_profit_percent: None,
                    profit_protection_activation_percent: None,
                    profit_protection_giveback_percent: None,
                    profit_protection_minimum_lock_percent: None,
                    profit_protection_peak_return_percent: None,
                    strategy_active_seen: false,
                    strategy_exit_requested: false,
                });
            }
        }
    }


    pub fn track_scout_shadow(
        &mut self,
        sample_id: String,
        symbol: String,
        strategy: String,
        timestamp: u64,
        direction: String,
        entry_price: f64,
        stop_loss_percent: f64,
        take_profit_percent: f64,
        profit_protection_activation_percent: f64,
        profit_protection_giveback_percent: f64,
        profit_protection_minimum_lock_percent: f64,
        scout_parameter_key: Option<String>,
        scout_parameter_value: Option<f64>,
        scout_minimum_hold_days: Option<f64>,
        scout_maximum_hold_days: Option<f64>,
    ) -> bool {
        if sample_id.is_empty() || strategy.is_empty()
            || !entry_price.is_finite() || entry_price <= 0.0
            || !stop_loss_percent.is_finite() || stop_loss_percent <= 0.0
            || !take_profit_percent.is_finite() || take_profit_percent <= 0.0
            || !profit_protection_activation_percent.is_finite() || profit_protection_activation_percent < 0.0
            || !profit_protection_giveback_percent.is_finite() || profit_protection_giveback_percent < 0.0
            || !profit_protection_minimum_lock_percent.is_finite() || profit_protection_minimum_lock_percent < 0.0
            || !matches!(direction.as_str(), "bullish" | "bearish")
            || scout_parameter_key.is_some() != scout_parameter_value.is_some()
            || scout_parameter_value.is_some_and(|value| !value.is_finite())
            || scout_minimum_hold_days.is_some_and(|days| !days.is_finite() || days <= 0.0)
            || scout_maximum_hold_days.is_some_and(|days| !days.is_finite() || days <= 0.0)
            || self.completed_scout_shadow_ids.contains(&sample_id)
            || self.pending.iter().any(|pending| pending.sample_id.as_deref() == Some(&sample_id))
        {
            return false;
        }
        let quote = self.best_quote.filter(|(quote_time, bid, ask)| {
            *quote_time <= timestamp
                && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                && bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid
        });
        let entry_spread = quote.map_or(0.0, |(_, bid, ask)| ask - bid);
        self.pending.push_back(PendingOutcome {
            symbol,
            timestamp,
            base_price: entry_price,
            direction,
            strength: 1.0,
            horizon_seconds: 0,
            entry_bid: Some(entry_price - entry_spread * 0.5),
            entry_ask: Some(entry_price + entry_spread * 0.5),
            trend_5m_volatility_units: None,
            features: None,
            score: 0.0,
            ranging_probability: None,
            entry_quote_timestamp: quote.map(|(quote_time, _, _)| quote_time),
            taker_fee_rate: configured_signal_taker_fee_rate(),
            first_long_positive_net_timestamp: None,
            first_short_positive_net_timestamp: None,
            sample_id: Some(sample_id),
            strategy: Some(strategy),
            scout_parameter_key,
            scout_parameter_value,
            scout_minimum_hold_days,
            scout_maximum_hold_days,
            stop_loss_percent: Some(stop_loss_percent),
            take_profit_percent: Some(take_profit_percent),
            profit_protection_activation_percent: Some(profit_protection_activation_percent),
            profit_protection_giveback_percent: Some(profit_protection_giveback_percent),
            profit_protection_minimum_lock_percent: Some(profit_protection_minimum_lock_percent),
            profit_protection_peak_return_percent: None,
            strategy_active_seen: false,
            strategy_exit_requested: false,
        });
        true
    }

    pub fn update_scout_strategy_state(
        &mut self,
        active_strategy_symbols: &BTreeSet<(String, String)>,
        active_parameter_profiles: &BTreeSet<(String, String, String, String)>,
        enabled_strategies: &BTreeSet<String>,
    ) {
        for pending in self.pending.iter_mut().filter(|pending| pending.sample_id.is_some()) {
            let Some(strategy) = pending.strategy.as_deref() else { continue; };
            if !enabled_strategies.contains(strategy) {
                pending.strategy_exit_requested = true;
                continue;
            }
            let active = match (pending.scout_parameter_key.as_deref(), pending.scout_parameter_value) {
                (Some(key), Some(value)) => active_parameter_profiles.contains(&(
                    strategy.to_owned(), pending.symbol.clone(), key.to_owned(), format!("{value:.8}"),
                )),
                _ => active_strategy_symbols.contains(&(strategy.to_owned(), pending.symbol.clone())),
            };
            if active {
                pending.strategy_active_seen = true;
            } else if pending.strategy_active_seen {
                pending.strategy_exit_requested = true;
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
                let midpoint = bid + (ask - bid) * 0.5;
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

#[cfg(test)]
mod scout_shadow_tests {
    use super::*;

    #[test]
    fn scout_shadow_uses_stop_target_and_strategy_invalidation_instead_of_one_hour() {
        let mut engine = SignalEngine::default();
        engine.on_book(1_000, 99.9, 100.1, &[1.0], &[1.0], &[(99.9, 1.0)], &[(100.1, 1.0)]);
        engine.on_price(1_000, 100.0, 0.0);
        assert!(engine.track_scout_shadow(
            "shadow-1".into(), "BTC-USDT".into(), "donchian-20-btc200".into(),
            1_000, "bullish".into(), 100.0, 0.5, 0.8, 0.15, 0.1, 0.05, None, None, None, None,
        ));
        engine.on_book(3_602_000, 100.0, 100.2, &[1.0], &[1.0], &[(100.0, 1.0)], &[(100.2, 1.0)]);
        assert!(engine.on_price(3_602_000, 100.1, 0.0).is_empty());

        let active = BTreeSet::from([("donchian-20-btc200".into(), "BTC-USDT".into())]);
        let enabled = BTreeSet::from(["donchian-20-btc200".into()]);
        engine.update_scout_strategy_state(&active, &BTreeSet::new(), &enabled);
        engine.update_scout_strategy_state(&BTreeSet::new(), &BTreeSet::new(), &enabled);
        engine.on_book(3_603_000, 100.0, 100.2, &[1.0], &[1.0], &[(100.0, 1.0)], &[(100.2, 1.0)]);
        let outcomes = engine.on_price(3_603_000, 100.1, 0.0);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].evaluation.horizon_seconds, 3_602);
        assert_eq!(outcomes[0].evaluation.signal_timestamp, 1_000);
        assert!(!engine.track_scout_shadow(
            "shadow-1".into(), "BTC-USDT".into(), "donchian-20-btc200".into(),
            1_000, "bullish".into(), 100.0, 0.5, 0.8, 0.15, 0.1, 0.05, None, None, None, None,
        ));
    }

    #[test]
    fn parameter_probe_exit_state_matches_the_exact_profile_variant() {
        let mut engine = SignalEngine::default();
        assert!(engine.track_scout_shadow(
            "probe-1".into(), "ETH-USDT".into(), "rsi2-oversold-rebound".into(),
            1_000, "bullish".into(), 100.0, 0.5, 0.8, 0.15, 0.1, 0.05,
            Some("rsi_exit_level".into()), Some(65.0), None, Some(5.0),
        ));
        let base_active = BTreeSet::from([("rsi2-oversold-rebound".into(), "ETH-USDT".into())]);
        let enabled = BTreeSet::from(["rsi2-oversold-rebound".into()]);
        engine.update_scout_strategy_state(&base_active, &BTreeSet::new(), &enabled);
        assert!(!engine.pending[0].strategy_active_seen);
        let active_variant = BTreeSet::from([(
            "rsi2-oversold-rebound".into(), "ETH-USDT".into(),
            "rsi_exit_level".into(), "65.00000000".into(),
        )]);
        engine.update_scout_strategy_state(&BTreeSet::new(), &active_variant, &enabled);
        assert!(engine.pending[0].strategy_active_seen);
        engine.update_scout_strategy_state(&BTreeSet::new(), &BTreeSet::new(), &enabled);
        assert!(engine.pending[0].strategy_exit_requested);
    }

    #[test]
    fn probe_hold_parameters_control_simulated_exit_timing() {
        let mut engine = SignalEngine::default();
        engine.on_book(1_000, 99.9, 100.1, &[1.0], &[1.0], &[(99.9, 1.0)], &[(100.1, 1.0)]);
        engine.on_price(1_000, 100.0, 0.0);
        assert!(engine.track_scout_shadow(
            "minimum-hold".into(), "BTC-USDT".into(), "donchian-20-btc200-hold3".into(),
            1_000, "bullish".into(), 100.0, 0.5, 0.8, 0.15, 0.1, 0.05,
            Some("minimum_hold_days".into()), Some(1.0), Some(1.0), None,
        ));
        let enabled = BTreeSet::from(["donchian-20-btc200-hold3".into()]);
        let active_variant = BTreeSet::from([(
            "donchian-20-btc200-hold3".into(), "BTC-USDT".into(),
            "minimum_hold_days".into(), "1.00000000".into(),
        )]);
        engine.update_scout_strategy_state(&BTreeSet::new(), &active_variant, &enabled);
        engine.update_scout_strategy_state(&BTreeSet::new(), &BTreeSet::new(), &enabled);

        let half_day = 43_201_000;
        engine.on_book(half_day, 100.0, 100.2, &[1.0], &[1.0], &[(100.0, 1.0)], &[(100.2, 1.0)]);
        assert!(engine.on_price(half_day, 100.1, 0.0).is_empty());
        let one_day = 86_401_000;
        engine.on_book(one_day, 100.0, 100.2, &[1.0], &[1.0], &[(100.0, 1.0)], &[(100.2, 1.0)]);
        assert_eq!(engine.on_price(one_day, 100.1, 0.0).len(), 1);

        let mut maximum_hold = SignalEngine::default();
        maximum_hold.on_book(1_000, 99.9, 100.1, &[1.0], &[1.0], &[(99.9, 1.0)], &[(100.1, 1.0)]);
        maximum_hold.on_price(1_000, 100.0, 0.0);
        assert!(maximum_hold.track_scout_shadow(
            "maximum-hold".into(), "ETH-USDT".into(), "rsi2-oversold-rebound".into(),
            1_000, "bullish".into(), 100.0, 0.5, 0.8, 0.15, 0.1, 0.05,
            Some("maximum_hold_days".into()), Some(1.0), None, Some(1.0),
        ));
        maximum_hold.on_book(one_day, 100.0, 100.2, &[1.0], &[1.0], &[(100.0, 1.0)], &[(100.2, 1.0)]);
        assert_eq!(maximum_hold.on_price(one_day, 100.1, 0.0).len(), 1);
    }

    #[test]
    fn tight_stop_target_scenario_exits_before_standard_scenario() {
        let mut pending = PendingOutcome {
            symbol: "BTC-USDT".into(), timestamp: 1_000, base_price: 100.0,
            direction: "bullish".into(), strength: 1.0, horizon_seconds: 0,
            entry_bid: Some(99.9), entry_ask: Some(100.1), trend_5m_volatility_units: None,
            features: None, score: 0.0, ranging_probability: None, entry_quote_timestamp: Some(1_000),
            taker_fee_rate: 0.001, first_long_positive_net_timestamp: None,
            first_short_positive_net_timestamp: None, sample_id: Some("tight".into()),
            strategy: Some("donchian-20-btc200".into()), scout_parameter_key: None, scout_parameter_value: None,
            scout_minimum_hold_days: None, scout_maximum_hold_days: None,
            stop_loss_percent: Some(0.25),
            take_profit_percent: Some(0.4), profit_protection_activation_percent: Some(0.15),
            profit_protection_giveback_percent: Some(0.1), profit_protection_minimum_lock_percent: Some(0.05),
            profit_protection_peak_return_percent: None, strategy_active_seen: false, strategy_exit_requested: false,
        };
        assert!(scout_shadow_stop_or_target_hit(&mut pending, Some((2_000, 101.0, 101.2))));
        let mut standard = PendingOutcome { take_profit_percent: Some(0.8), ..pending };
        assert!(!scout_shadow_stop_or_target_hit(&mut standard, Some((2_000, 101.0, 101.2))));
    }
}
