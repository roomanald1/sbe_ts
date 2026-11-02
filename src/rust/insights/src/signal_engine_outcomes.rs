const MAX_SCOUT_SHADOW_DURATION_MS: u64 = 7 * 86_400_000;

fn scout_shadow_stop_or_target_hit(
    pending: &mut PendingOutcome,
    quote: Option<(u64, f64, f64)>,
) -> bool {
    let Some((_, bid, ask)) = quote else { return false; };
    let (entry, exit) = match pending.direction.as_str() {
        "bullish" => (pending.entry_ask, bid),
        "bearish" => (pending.entry_bid, ask),
        _ => return false,
    };
    let (Some(entry), Some(stop), Some(target)) = (
        entry,
        pending.stop_loss_percent,
        pending.take_profit_percent,
    ) else {
        return false;
    };
    let Some(return_percent) = directional_net_return(
        &pending.direction,
        entry,
        exit,
        pending.taker_fee_rate,
    ) else {
        return false;
    };
    if let Some(activation) = pending.profit_protection_activation_percent {
        if return_percent >= activation {
            pending.profit_protection_peak_return_percent = Some(
                pending.profit_protection_peak_return_percent
                    .map_or(return_percent, |peak| peak.max(return_percent)),
            );
        }
    }
    let protection_exit = pending.profit_protection_peak_return_percent.is_some_and(|peak| {
        let giveback = pending.profit_protection_giveback_percent.unwrap_or_default();
        let minimum_lock = pending.profit_protection_minimum_lock_percent.unwrap_or_default();
        return_percent <= (peak - giveback).max(minimum_lock)
    });
    return_percent <= -stop || return_percent >= target || protection_exit
}

impl SignalEngine {
    pub fn live_snapshot_due(&mut self, timestamp: u64) -> bool {
        let due = self.last_live_snapshot_at.map_or(true, |last| {
            timestamp < last || timestamp.saturating_sub(last) >= LIVE_SIGNAL_SNAPSHOT_INTERVAL_MS
        });
        if due {
            self.last_live_snapshot_at = Some(timestamp);
        }
        due
    }

    /// Check the research cadence without consuming its slot.
    pub fn research_sample_is_due(&self, timestamp: u64) -> bool {
        self.last_research_sample_at.map_or(true, |last| {
            timestamp < last || timestamp.saturating_sub(last) >= RESEARCH_SAMPLE_INTERVAL_MS
        })
    }

    /// Gate research feature extraction and persistence, independently of live snapshots.
    pub fn research_sample_due(&mut self, timestamp: u64) -> bool {
        let due = self.research_sample_is_due(timestamp);
        if due {
            self.last_research_sample_at = Some(timestamp);
        }
        due
    }

    pub fn on_price(
        &mut self,
        timestamp: u64,
        price: f64,
        volatility_1m: f64,
    ) -> Vec<CompletedSignalOutcome> {
        if !price.is_finite() || price <= 0.0 {
            return Vec::new();
        }
        if self
            .prices
            .back()
            .is_some_and(|sample| timestamp < sample.0)
        {
            return Vec::new();
        }
        self.prices.push_back((timestamp, price));
        self.volatility_1m = volatility_1m.is_finite().then_some(volatility_1m);
        self.prune(timestamp);
        let mut outcomes = Vec::new();
        let mut remaining = VecDeque::new();
        while let Some(mut pending) = self.pending.pop_front() {
            if pending.first_long_positive_net_timestamp.is_none()
                || pending.first_short_positive_net_timestamp.is_none()
            {
                let quote = self.best_quote.filter(|(quote_time, _, _)| {
                    *quote_time <= timestamp
                        && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                });
                if let Some((_, exit_bid, exit_ask)) = quote {
                    if pending.first_long_positive_net_timestamp.is_none()
                        && pending.entry_ask.is_some_and(|entry| {
                            directional_net_return(
                                "bullish",
                                entry,
                                exit_bid,
                                pending.taker_fee_rate,
                            )
                            .is_some_and(|net| net > 0.0)
                        })
                    {
                        pending.first_long_positive_net_timestamp = Some(timestamp);
                    }
                    if pending.first_short_positive_net_timestamp.is_none()
                        && pending.entry_bid.is_some_and(|entry| {
                            directional_net_return(
                                "bearish",
                                entry,
                                exit_ask,
                                pending.taker_fee_rate,
                            )
                            .is_some_and(|net| net > 0.0)
                        })
                    {
                        pending.first_short_positive_net_timestamp = Some(timestamp);
                    }
                }
            }
            let current_quote = self.best_quote.filter(|(quote_time, _, _)| {
                *quote_time <= timestamp
                    && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
            });
            let is_scout_shadow = pending.sample_id.is_some();
            let shadow_age_ms = timestamp.saturating_sub(pending.timestamp);
            let minimum_hold_active = pending.scout_minimum_hold_days
                .is_some_and(|days| shadow_age_ms < (days * 86_400_000.0) as u64);
            let maximum_hold_elapsed = pending.scout_maximum_hold_days
                .is_some_and(|days| shadow_age_ms >= (days * 86_400_000.0) as u64);
            let maximum_shadow_duration_elapsed =
                is_scout_shadow && shadow_age_ms >= MAX_SCOUT_SHADOW_DURATION_MS;
            let shadow_exit = is_scout_shadow
                && current_quote.is_some()
                && ((pending.strategy_exit_requested && !minimum_hold_active)
                    || maximum_hold_elapsed
                    || maximum_shadow_duration_elapsed
                    || scout_shadow_stop_or_target_hit(&mut pending, current_quote));
            let fixed_horizon_exit = !is_scout_shadow
                && timestamp
                    >= pending
                        .timestamp
                        .saturating_add(pending.horizon_seconds * 1000);
            if shadow_exit || fixed_horizon_exit {
                let evaluation_horizon_seconds = if is_scout_shadow {
                    timestamp.saturating_sub(pending.timestamp).saturating_div(1000).max(1)
                } else {
                    pending.horizon_seconds
                };
                let change = (price / pending.base_price - 1.0) * 100.0;
                let direction_match = match pending.direction.as_str() {
                    "bullish" => change > 0.0,
                    "bearish" => change < 0.0,
                    _ => false,
                };
                let exit_quote = self.best_quote.filter(|(quote_time, _, _)| {
                    *quote_time <= timestamp
                        && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                });
                let fee_rate = pending.taker_fee_rate;
                let net_return_percent = match (
                    pending.direction.as_str(),
                    pending.entry_bid,
                    pending.entry_ask,
                    exit_quote,
                ) {
                    ("bullish", _, Some(entry_ask), Some((_, exit_bid, _))) => {
                        directional_net_return("bullish", entry_ask, exit_bid, fee_rate)
                    }
                    ("bearish", Some(entry_bid), _, Some((_, _, exit_ask))) => {
                        directional_net_return("bearish", entry_bid, exit_ask, fee_rate)
                    }
                    _ => None,
                };
                let gross_directional_return = match pending.direction.as_str() {
                    "bullish" => Some(change),
                    "bearish" => Some(-change),
                    _ => None,
                };
                let execution_cost_percent = gross_directional_return
                    .zip(net_return_percent)
                    .map(|(gross, net)| gross - net);
                let trend_5m_direction = pending
                    .trend_5m_volatility_units
                    .filter(|value| value.abs() >= 0.25)
                    .map(|value| if value > 0.0 { "bullish" } else { "bearish" });
                let trend_5m_direction_match = trend_5m_direction.map(|direction| {
                    if direction == "bullish" {
                        change > 0.0
                    } else {
                        change < 0.0
                    }
                });
                let trend_5m_net_return_percent = match (
                    trend_5m_direction,
                    pending.entry_bid,
                    pending.entry_ask,
                    exit_quote,
                ) {
                    (Some("bullish"), _, Some(entry_ask), Some((_, exit_bid, _))) => {
                        directional_net_return("bullish", entry_ask, exit_bid, fee_rate)
                    }
                    (Some("bearish"), Some(entry_bid), _, Some((_, _, exit_ask))) => {
                        directional_net_return("bearish", entry_bid, exit_ask, fee_rate)
                    }
                    _ => None,
                };
                let legacy = SignalOutcome {
                    record_type: "outcome".into(),
                    symbol: pending.symbol.clone(),
                    signal_timestamp: pending.timestamp,
                    evaluated_timestamp: timestamp,
                    horizon_seconds: evaluation_horizon_seconds,
                    return_percent: change,
                    direction_match,
                    direction: pending.direction.clone(),
                    strength: pending.strength,
                    net_return_percent,
                    execution_cost_percent,
                    trend_5m_volatility_units: pending.trend_5m_volatility_units,
                    trend_5m_direction_match,
                    trend_5m_net_return_percent,
                    features: pending.features,
                };

                let mut path_high = pending.base_price;
                let mut path_high_timestamp = pending.timestamp;
                let mut path_low = pending.base_price;
                let mut path_low_timestamp = pending.timestamp;
                for (sample_time, sample_price) in self.prices.iter().filter(|(sample_time, _)| {
                    *sample_time >= pending.timestamp && *sample_time <= timestamp
                }) {
                    if *sample_price > path_high {
                        path_high = *sample_price;
                        path_high_timestamp = *sample_time;
                    }
                    if *sample_price < path_low {
                        path_low = *sample_price;
                        path_low_timestamp = *sample_time;
                    }
                }
                let exit_quote = self.best_quote.filter(|(quote_time, _, _)| {
                    *quote_time <= timestamp
                        && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                });
                let (exit_bid, exit_ask, exit_quote_timestamp) = exit_quote
                    .map_or((None, None, None), |(quote_time, bid, ask)| {
                        (Some(bid), Some(ask), Some(quote_time))
                    });
                let long_gross_percent = (price / pending.base_price - 1.0) * 100.0;
                let short_gross_percent =
                    ((pending.base_price - price) / pending.base_price) * 100.0;
                let long_net_percent = pending.entry_ask.zip(exit_bid).and_then(|(entry, exit)| {
                    directional_net_return("bullish", entry, exit, pending.taker_fee_rate)
                });
                let short_net_percent =
                    pending.entry_bid.zip(exit_ask).and_then(|(entry, exit)| {
                        directional_net_return("bearish", entry, exit, pending.taker_fee_rate)
                    });
                let evaluation = SignalEvaluation {
                    symbol: pending.symbol,
                    signal_timestamp: pending.timestamp,
                    evaluated_timestamp: timestamp,
                    horizon_seconds: evaluation_horizon_seconds,
                    signal_direction: pending.direction,
                    signal_score: pending.score,
                    ranging_probability: pending.ranging_probability,
                    reference_entry_price: pending.base_price,
                    reference_exit_price: price,
                    entry_bid: pending.entry_bid,
                    entry_ask: pending.entry_ask,
                    entry_quote_timestamp: pending.entry_quote_timestamp,
                    exit_bid,
                    exit_ask,
                    exit_quote_timestamp,
                    taker_fee_rate: pending.taker_fee_rate,
                    long_gross_percent,
                    short_gross_percent,
                    long_net_percent,
                    short_net_percent,
                    path_high,
                    path_high_timestamp,
                    path_low,
                    path_low_timestamp,
                    long_time_to_positive_net_seconds: pending
                        .first_long_positive_net_timestamp
                        .map(|first| first.saturating_sub(pending.timestamp) / 1000),
                    short_time_to_positive_net_seconds: pending
                        .first_short_positive_net_timestamp
                        .map(|first| first.saturating_sub(pending.timestamp) / 1000),
                };
                let scout_shadow_id = pending.sample_id.clone();
                if let Some(sample_id) = scout_shadow_id.as_ref() {
                    self.completed_scout_shadow_ids.insert(sample_id.clone());
                }
                outcomes.push(CompletedSignalOutcome { legacy, evaluation, scout_shadow_id });
            } else {
                remaining.push_back(pending);
            }
        }
        self.pending = remaining;
        outcomes
    }

    pub fn return_context(&self, timestamp: u64) -> (Option<f64>, Option<f64>) {
        let fresh = self.prices.back().is_some_and(|(sample_time, _)| {
            timestamp.saturating_sub(*sample_time) <= MAX_BOOK_AGE_MS
        });
        if !fresh {
            return (None, None);
        }
        (
            self.price_return(timestamp.saturating_sub(60_000)),
            self.price_return(timestamp.saturating_sub(300_000)),
        )
    }
}
