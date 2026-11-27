fn executable_scout_price(
    direction: &str,
    entry_bid: Option<f64>,
    entry_ask: Option<f64>,
    bid: f64,
    ask: f64,
    entry_quote_timestamp: Option<u64>,
) -> Option<(f64, f64)> {
    if !bid.is_finite()
        || !ask.is_finite()
        || bid <= 0.0
        || ask < bid
        || entry_quote_timestamp.is_none()
    {
        return None;
    }
    match direction {
        "bullish" => entry_ask
            .filter(|entry| entry.is_finite() && *entry > 0.0)
            .map(|entry| (entry, bid)),
        "bearish" => entry_bid
            .filter(|entry| entry.is_finite() && *entry > 0.0)
            .map(|entry| (entry, ask)),
        _ => None,
    }
}

fn scout_return_percent(direction: &str, entry: f64, execution_price: f64) -> f64 {
    match direction {
        "bullish" => (execution_price / entry - 1.0) * 100.0,
        "bearish" => ((entry - execution_price) / entry) * 100.0,
        _ => 0.0,
    }
}

fn apply_scout_path_sample(
    path: &mut ScoutPathAccumulator,
    sample: &ScoutPathSample,
    return_percent: f64,
    retention_limit: usize,
) {
    if path.retained_samples.back().is_some_and(|previous| {
        (previous.path_epoch, previous.sequence) >= (sample.path_epoch, sample.sequence)
    }) {
        return;
    }
    if let Some(previous) = path.last_source_timestamp {
        if sample.timestamp < previous {
            path.source_timestamps_monotonic = false;
        } else {
            path.maximum_gap_ms = path.maximum_gap_ms.max(sample.timestamp - previous);
        }
    } else {
        path.first_source_timestamp = Some(sample.timestamp);
    }
    path.sample_count = path.sample_count.saturating_add(1);
    path.last_source_timestamp = Some(sample.timestamp);
    path.last_sequence = sample.sequence;
    path.latest_freshness_ms = Some(sample.freshness_ms);
    path.maximum_favorable_excursion_percent = Some(
        path.maximum_favorable_excursion_percent
            .map_or(return_percent, |current| current.max(return_percent)),
    );
    path.maximum_adverse_excursion_percent = Some(
        path.maximum_adverse_excursion_percent
            .map_or(return_percent, |current| current.min(return_percent)),
    );
    if retention_limit > 0 && path.retained_samples.len() == retention_limit {
        path.retained_samples.pop_front();
        path.truncated = true;
    }
    if retention_limit > 0 {
        path.retained_samples.push_back(sample.clone());
    }
}

fn feature_vector(components: &SignalComponents) -> Option<[f64; 4]> {
    let values = [
        components
            .trend_1m_percent
            .map(|v| (v / 0.25).clamp(-4.0, 4.0)),
        components
            .momentum_10s_percent
            .map(|v| (v / 0.12).clamp(-4.0, 4.0)),
        components.trade_flow_imbalance,
        components.book_imbalance,
    ];
    values
        .iter()
        .all(Option::is_some)
        .then(|| values.map(Option::unwrap))
}

impl SignalEngine {
    const MAX_RETAINED_SCOUT_PATH_SAMPLES: usize = 4_096;

    pub fn mark_scout_path_interrupted(&mut self, sample_id: &str) -> bool {
        let mut marked = false;
        for pending in self
            .pending
            .iter_mut()
            .filter(|pending| pending.sample_id.as_deref() == Some(sample_id))
        {
            pending.scout_path.interrupted_by_process_restart = true;
            marked = true;
        }
        marked
    }

    pub fn mark_scout_path_capture_failed(&mut self, sample_id: &str) -> bool {
        let mut marked = false;
        for pending in self
            .pending
            .iter_mut()
            .filter(|pending| pending.sample_id.as_deref() == Some(sample_id))
        {
            pending.scout_path.capture_failed = true;
            marked = true;
        }
        marked
    }

    pub fn set_scout_path_epoch(&mut self, sample_id: &str, epoch: u64) {
        for pending in self
            .pending
            .iter_mut()
            .filter(|pending| pending.sample_id.as_deref() == Some(sample_id))
        {
            pending.scout_path.path_epoch = epoch;
        }
    }

    pub fn has_pending_scout(&self, sample_id: &str) -> bool {
        self.pending
            .iter()
            .any(|pending| pending.sample_id.as_deref() == Some(sample_id))
    }

    pub fn scout_entry_quote(&self, timestamp: u64, entry_price: f64) -> Option<(f64, f64, u64)> {
        self.best_quote
            .filter(|(quote_time, bid, ask)| {
                *quote_time <= timestamp
                    && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                    && bid.is_finite()
                    && ask.is_finite()
                    && *bid > 0.0
                    && *ask >= *bid
            })
            .map(|(quote_time, bid, ask)| {
                let half_spread = (ask - bid) * 0.5;
                (
                    entry_price - half_spread,
                    entry_price + half_spread,
                    quote_time,
                )
            })
    }

    /// Restore only observations that were durably captured. The restart flag
    /// remains set so later observations cannot make this path complete.
    pub fn restore_scout_path_samples(
        &mut self,
        sample_id: &str,
        samples: &[ScoutPathSample],
        truncated: bool,
        freshness_ms: Option<u64>,
        persisted_sample_count: u64,
        entry_quote: Option<(f64, f64, u64)>,
    ) {
        let mut ordered = samples
            .iter()
            .filter(|sample| {
                sample.decision_id == sample_id
                    && self
                        .pending
                        .iter()
                        .find(|pending| pending.sample_id.as_deref() == Some(sample_id))
                        .is_some_and(|pending| sample.symbol == pending.symbol)
            })
            .collect::<Vec<_>>();
        ordered.sort_by_key(|sample| (sample.path_epoch, sample.sequence));
        for pending in self
            .pending
            .iter_mut()
            .filter(|pending| pending.sample_id.as_deref() == Some(sample_id))
        {
            let path = &mut pending.scout_path;
            if let Some((entry_bid, entry_ask, quote_timestamp)) = entry_quote {
                pending.entry_bid = Some(entry_bid);
                pending.entry_ask = Some(entry_ask);
                pending.entry_quote_timestamp = Some(quote_timestamp);
            }
            let mut last_restored = None;
            for sample in &ordered {
                if sample.timestamp < pending.timestamp
                    || (pending.scout_observation_only
                        && sample.timestamp
                            > pending
                                .timestamp
                                .saturating_add(pending.horizon_seconds.saturating_mul(1_000)))
                {
                    continue;
                }
                let Some((entry, execution_price)) = executable_scout_price(
                    pending.direction.as_str(),
                    pending.entry_bid,
                    pending.entry_ask,
                    sample.bid,
                    sample.ask,
                    pending.entry_quote_timestamp,
                ) else {
                    continue;
                };
                let return_percent =
                    scout_return_percent(pending.direction.as_str(), entry, execution_price);
                apply_scout_path_sample(
                    path,
                    sample,
                    return_percent,
                    if pending.scout_observation_only {
                        0
                    } else {
                        Self::MAX_RETAINED_SCOUT_PATH_SAMPLES
                    },
                );
                path.latest_sample = Some((**sample).clone());
                last_restored = Some(*sample);
            }
            let restored_horizon_sample_count = last_restored.map(|sample| sample.sample_count);
            if let Some(last) = last_restored {
                path.first_source_timestamp = last.first_source_timestamp;
                path.maximum_gap_ms = path.maximum_gap_ms.max(last.maximum_gap_ms);
                path.source_timestamps_monotonic &= last.source_timestamps_monotonic;
                path.maximum_favorable_excursion_percent = last.maximum_favorable_excursion_percent;
                path.maximum_adverse_excursion_percent = last.maximum_adverse_excursion_percent;
                path.sample_count = path.sample_count.max(last.sample_count);
                path.last_sequence = last.sequence;
                path.last_source_timestamp = Some(last.timestamp);
                path.latest_sample = Some((*last).clone());
            }
            path.truncated |= truncated;
            let restored_count = if pending.scout_observation_only {
                restored_horizon_sample_count.unwrap_or(0)
            } else {
                persisted_sample_count
            };
            path.sample_count = path.sample_count.max(restored_count);
            path.latest_freshness_ms = freshness_ms;
            path.interrupted_by_process_restart = true;
        }
    }

    /// Returns one newly observed sample per active Scout identity. Multiple
    /// pending horizons share the same source path and are deduplicated here.
    pub fn latest_scout_path_samples(
        &self,
        timestamp: u64,
        bid: f64,
        ask: f64,
        freshness_ms: u64,
    ) -> Vec<ScoutPathSample> {
        let mut latest = BTreeMap::<String, ScoutPathSample>::new();
        for pending in self
            .pending
            .iter()
            .filter(|pending| pending.sample_id.is_some())
        {
            let Some(sample) = pending.scout_path.latest_sample.as_ref() else {
                continue;
            };
            if sample.timestamp != timestamp || sample.bid != bid || sample.ask != ask {
                continue;
            }
            let mut sample = sample.clone();
            sample.freshness_ms = freshness_ms;
            latest.insert(sample.decision_id.clone(), sample);
        }
        latest.into_values().collect()
    }

    pub fn track_counterfactual(
        &mut self,
        symbol: String,
        timestamp: u64,
        direction: String,
        entry_price: f64,
    ) -> bool {
        if symbol.is_empty()
            || !matches!(direction.as_str(), "bullish" | "bearish")
            || !entry_price.is_finite()
            || entry_price <= 0.0
            || self.pending.iter().any(|pending| {
                pending.sample_id.is_none()
                    && pending.symbol == symbol
                    && pending.timestamp == timestamp
                    && pending.direction == direction
            })
        {
            return false;
        }
        let quote = self.best_quote.filter(|(quote_time, bid, ask)| {
            *quote_time <= timestamp
                && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                && bid.is_finite()
                && ask.is_finite()
                && *bid > 0.0
                && *ask >= *bid
        });
        let entry_spread = quote.map_or(0.0, |(_, bid, ask)| ask - bid);
        for horizon_seconds in [60, 300, 900, 3_600] {
            self.pending.push_back(PendingOutcome {
                symbol: symbol.clone(),
                timestamp,
                base_price: entry_price,
                direction: direction.clone(),
                strength: 1.0,
                horizon_seconds,
                entry_bid: Some(entry_price - entry_spread * 0.5),
                entry_ask: Some(entry_price + entry_spread * 0.5),
                trend_5m_volatility_units: None,
                features: None,
                score: 0.0,
                ranging_probability: None,
                entry_quote_timestamp: quote.map(|(quote_time, _, _)| quote_time),
                scout_path: ScoutPathAccumulator {
                    source_timestamps_monotonic: true,
                    ..Default::default()
                },
                taker_fee_rate: configured_signal_taker_fee_rate(),
                first_long_positive_net_timestamp: None,
                first_short_positive_net_timestamp: None,
                sample_id: None,
                strategy: None,
                scout_parameter_key: None,
                scout_parameter_value: None,
                scout_minimum_hold_days: None,
                scout_maximum_hold_days: None,
                scout_maximum_duration_ms: None,
                scout_observation_only: false,
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
        true
    }

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
                    scout_path: ScoutPathAccumulator {
                        source_timestamps_monotonic: true,
                        ..Default::default()
                    },
                    taker_fee_rate: configured_signal_taker_fee_rate(),
                    first_long_positive_net_timestamp: None,
                    first_short_positive_net_timestamp: None,
                    sample_id: None,
                    strategy: None,
                    scout_parameter_key: None,
                    scout_parameter_value: None,
                    scout_minimum_hold_days: None,
                    scout_maximum_hold_days: None,
                    scout_maximum_duration_ms: None,
                    scout_observation_only: false,
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

    pub fn track_scout(
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
        self.track_scout_with_maximum_duration(
            sample_id,
            symbol,
            strategy,
            timestamp,
            direction,
            entry_price,
            stop_loss_percent,
            take_profit_percent,
            profit_protection_activation_percent,
            profit_protection_giveback_percent,
            profit_protection_minimum_lock_percent,
            scout_parameter_key,
            scout_parameter_value,
            scout_minimum_hold_days,
            scout_maximum_hold_days,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn track_scout_with_maximum_duration(
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
        scout_maximum_duration_ms: Option<u64>,
    ) -> bool {
        if sample_id.is_empty()
            || strategy.is_empty()
            || !entry_price.is_finite()
            || entry_price <= 0.0
            || !stop_loss_percent.is_finite()
            || stop_loss_percent <= 0.0
            || !take_profit_percent.is_finite()
            || take_profit_percent <= 0.0
            || !profit_protection_activation_percent.is_finite()
            || profit_protection_activation_percent < 0.0
            || !profit_protection_giveback_percent.is_finite()
            || profit_protection_giveback_percent < 0.0
            || !profit_protection_minimum_lock_percent.is_finite()
            || profit_protection_minimum_lock_percent < 0.0
            || !matches!(direction.as_str(), "bullish" | "bearish")
            || scout_parameter_key.is_some() != scout_parameter_value.is_some()
            || scout_parameter_value.is_some_and(|value| !value.is_finite())
            || scout_minimum_hold_days.is_some_and(|days| !days.is_finite() || days <= 0.0)
            || scout_maximum_hold_days.is_some_and(|days| !days.is_finite() || days <= 0.0)
            || scout_maximum_duration_ms
                .is_some_and(|duration| duration == 0 || duration > 7 * 86_400_000)
            || self.completed_scout_ids.contains(&sample_id)
            || self
                .pending
                .iter()
                .any(|pending| pending.sample_id.as_deref() == Some(&sample_id))
        {
            return false;
        }
        let quote = self.best_quote.filter(|(quote_time, bid, ask)| {
            *quote_time <= timestamp
                && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                && bid.is_finite()
                && ask.is_finite()
                && *bid > 0.0
                && *ask >= *bid
        });
        let entry_spread = quote.map_or(0.0, |(_, bid, ask)| ask - bid);
        let scout_pending = PendingOutcome {
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
            scout_path: ScoutPathAccumulator {
                source_timestamps_monotonic: true,
                maximum_favorable_excursion_percent: quote.map(|_| 0.0),
                maximum_adverse_excursion_percent: quote.map(|_| 0.0),
                ..Default::default()
            },
            taker_fee_rate: configured_signal_taker_fee_rate(),
            first_long_positive_net_timestamp: None,
            first_short_positive_net_timestamp: None,
            sample_id: Some(sample_id),
            strategy: Some(strategy),
            scout_parameter_key,
            scout_parameter_value,
            scout_minimum_hold_days,
            scout_maximum_hold_days,
            scout_maximum_duration_ms,
            scout_observation_only: false,
            stop_loss_percent: Some(stop_loss_percent),
            take_profit_percent: Some(take_profit_percent),
            profit_protection_activation_percent: Some(profit_protection_activation_percent),
            profit_protection_giveback_percent: Some(profit_protection_giveback_percent),
            profit_protection_minimum_lock_percent: Some(profit_protection_minimum_lock_percent),
            profit_protection_peak_return_percent: None,
            strategy_active_seen: false,
            strategy_exit_requested: false,
        };
        self.pending.push_back(scout_pending.clone());
        for horizon_seconds in [60, 300, 900, 3_600] {
            let mut observation = scout_pending.clone();
            observation.horizon_seconds = horizon_seconds;
            observation.scout_observation_only = true;
            observation.scout_minimum_hold_days = None;
            observation.scout_maximum_hold_days = None;
            observation.scout_maximum_duration_ms = None;
            observation.stop_loss_percent = None;
            observation.take_profit_percent = None;
            observation.profit_protection_activation_percent = None;
            observation.profit_protection_giveback_percent = None;
            observation.profit_protection_minimum_lock_percent = None;
            self.pending.push_back(observation);
        }
        true
    }

    pub fn update_scout_strategy_state(
        &mut self,
        active_strategy_symbols: &BTreeSet<(String, String)>,
        active_parameter_profiles: &BTreeSet<(String, String, String, String)>,
        enabled_strategies: &BTreeSet<String>,
    ) {
        for pending in self
            .pending
            .iter_mut()
            .filter(|pending| pending.sample_id.is_some())
        {
            let Some(strategy) = pending.strategy.as_deref() else {
                continue;
            };
            if !enabled_strategies.contains(strategy) {
                pending.strategy_exit_requested = true;
                continue;
            }
            let active = match (
                pending.scout_parameter_key.as_deref(),
                pending.scout_parameter_value,
            ) {
                (Some(key), Some(value)) => active_parameter_profiles.contains(&(
                    strategy.to_owned(),
                    pending.symbol.clone(),
                    key.to_owned(),
                    format!("{value:.8}"),
                )),
                _ => {
                    active_strategy_symbols.contains(&(strategy.to_owned(), pending.symbol.clone()))
                }
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
        // Fill short-term mark history from active trade streams when ticker
        // updates are sparse. The sampler handles cross-stream reordering.
        self.record_price_sample(timestamp, price);
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
        self.on_book_with_receive_time(
            timestamp, timestamp, best_bid, best_ask, bid_sizes, ask_sizes, bid_levels, ask_levels,
        );
    }

    pub fn on_book_with_receive_time(
        &mut self,
        timestamp: u64,
        received_at_ms: u64,
        best_bid: f64,
        best_ask: f64,
        bid_sizes: &[f64],
        ask_sizes: &[f64],
        bid_levels: &[(f64, f64)],
        ask_levels: &[(f64, f64)],
    ) {
        self.book_input_provenance.received = self.book_input_provenance.received.saturating_add(1);
        self.book_input_provenance.last_source_timestamp_ms = Some(timestamp);
        self.book_input_provenance.last_received_at_ms = Some(received_at_ms);
        if best_bid.is_finite() && best_ask.is_finite() && best_bid > 0.0 && best_ask >= best_bid {
            // Feed streams can deliver older book events after newer ticker or
            // trade events. Keep observing source order for Scout completeness,
            // but never let a late event regress the quote used by snapshots.
            // No source sequence is available, so the first quote wins when
            // timestamps are equal.
            let current_timestamp = self
                .best_quote
                .map(|(current_timestamp, _, _)| current_timestamp);
            let advances_current_quote =
                current_timestamp.is_none_or(|current| timestamp > current);
            if advances_current_quote {
                self.best_quote = Some((timestamp, best_bid, best_ask));
                self.book_input_provenance.accepted =
                    self.book_input_provenance.accepted.saturating_add(1);
                self.book_input_provenance.last_accepted_source_timestamp_ms = Some(timestamp);
                self.book_input_provenance.last_accepted_at_ms = Some(received_at_ms);
            } else if current_timestamp == Some(timestamp) {
                self.book_input_provenance.equal_timestamp_rejected = self
                    .book_input_provenance
                    .equal_timestamp_rejected
                    .saturating_add(1);
            } else {
                self.book_input_provenance.older_timestamp_rejected = self
                    .book_input_provenance
                    .older_timestamp_rejected
                    .saturating_add(1);
            }
            for pending in self.pending.iter_mut().filter(|pending| {
                pending.sample_id.is_some()
                    && timestamp >= pending.timestamp
                    && (!pending.scout_observation_only
                        || timestamp
                            <= pending
                                .timestamp
                                .saturating_add(pending.horizon_seconds.saturating_mul(1_000)))
            }) {
                let path = &mut pending.scout_path;
                let Some((entry, execution_price)) = executable_scout_price(
                    pending.direction.as_str(),
                    pending.entry_bid,
                    pending.entry_ask,
                    best_bid,
                    best_ask,
                    pending.entry_quote_timestamp,
                ) else {
                    continue;
                };
                if path
                    .last_source_timestamp
                    .is_some_and(|previous| timestamp < previous)
                {
                    path.source_timestamps_monotonic = false;
                    continue;
                }
                let gap_ms = path
                    .last_source_timestamp
                    .map_or(0, |previous| timestamp.saturating_sub(previous));
                let sequence = path.last_sequence.saturating_add(1);
                let sample = ScoutPathSample {
                    decision_id: pending.sample_id.clone().unwrap_or_default(),
                    symbol: pending.symbol.clone(),
                    path_epoch: path.path_epoch,
                    sequence,
                    timestamp,
                    bid: best_bid,
                    ask: best_ask,
                    freshness_ms: 0,
                    gap_ms,
                    first_source_timestamp: path.first_source_timestamp,
                    maximum_gap_ms: path.maximum_gap_ms,
                    source_timestamps_monotonic: path.source_timestamps_monotonic,
                    maximum_favorable_excursion_percent: path.maximum_favorable_excursion_percent,
                    maximum_adverse_excursion_percent: path.maximum_adverse_excursion_percent,
                    sample_count: path.sample_count,
                    capture_complete: !path.capture_failed,
                };
                apply_scout_path_sample(
                    path,
                    &sample,
                    scout_return_percent(pending.direction.as_str(), entry, execution_price),
                    if pending.scout_observation_only {
                        0
                    } else {
                        Self::MAX_RETAINED_SCOUT_PATH_SAMPLES
                    },
                );
                // Observation horizons share the durable Scout path row set;
                // mark them truncated when that bounded set no longer covers
                // every observed event, even though they retain only summary
                // aggregates in memory.
                if path.sample_count > Self::MAX_RETAINED_SCOUT_PATH_SAMPLES as u64 {
                    path.truncated = true;
                }
                let mut sample = sample;
                sample.first_source_timestamp = path.first_source_timestamp;
                sample.maximum_gap_ms = path.maximum_gap_ms;
                sample.source_timestamps_monotonic = path.source_timestamps_monotonic;
                sample.maximum_favorable_excursion_percent =
                    path.maximum_favorable_excursion_percent;
                sample.maximum_adverse_excursion_percent = path.maximum_adverse_excursion_percent;
                sample.sample_count = path.sample_count;
                if let Some(retained) = path.retained_samples.back_mut() {
                    *retained = sample.clone();
                }
                path.latest_sample = Some(sample);
            }
            if !advances_current_quote {
                return;
            }
        } else {
            self.book_input_provenance.invalid_book_rejected = self
                .book_input_provenance
                .invalid_book_rejected
                .saturating_add(1);
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
            let sample = BookHistorySample {
                timestamp,
                bid: best_bid,
                ask: best_ask,
                bid_levels,
                ask_levels,
            };
            let should_record = self.book_history.back().is_none_or(|previous| {
                timestamp == previous.timestamp
                    || timestamp.saturating_sub(previous.timestamp)
                        >= BOOK_HISTORY_SAMPLE_INTERVAL_MS
            });
            if should_record {
                if self
                    .book_history
                    .back()
                    .is_some_and(|previous| timestamp == previous.timestamp)
                {
                    *self.book_history.back_mut().expect("history has a back") = sample;
                } else {
                    self.book_history.push_back(sample);
                }
            }
            while self.book_history.front().is_some_and(|sample| {
                sample.timestamp < timestamp.saturating_sub(60_000)
                    || self.book_history.len() > BOOK_HISTORY_SAMPLE_LIMIT
            }) {
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
mod scout_tests {
    use super::*;

    #[test]
    fn trade_prices_fill_momentum_history_when_ticker_updates_are_sparse_and_reordered() {
        let mut engine = SignalEngine::default();
        // A newer ticker can be processed before the trade stream catches up.
        engine.on_price(12_000, 100.12, 0.0);
        for second in 0_u32..=11 {
            let price = 100.0 + f64::from(second) * 0.01;
            engine.on_trade(u64::from(second) * 1_000, price, 1.0, true);
        }

        let snapshot = engine.snapshot("BTC-USDT", 12_000).unwrap();
        let momentum = snapshot
            .components
            .momentum_by_window
            .iter()
            .find(|feature| feature.window_seconds == 10)
            .and_then(|feature| feature.value);
        assert!(momentum.is_some());
    }

    #[test]
    fn late_book_events_do_not_regress_the_current_quote_or_book_state() {
        let mut engine = SignalEngine::default();
        engine.on_book(
            20_000,
            100.0,
            100.2,
            &[3.0],
            &[1.0],
            &[(100.0, 3.0)],
            &[(100.2, 1.0)],
        );
        engine.on_book(
            20_000,
            105.0,
            105.2,
            &[1.0],
            &[3.0],
            &[(105.0, 1.0)],
            &[(105.2, 3.0)],
        );
        engine.on_book(
            10_000,
            90.0,
            90.2,
            &[1.0],
            &[3.0],
            &[(90.0, 1.0)],
            &[(90.2, 3.0)],
        );

        assert_eq!(engine.best_quote, Some((20_000, 100.0, 100.2)));
        assert_eq!(engine.book_imbalance, Some(0.5));
        assert_eq!(engine.book_bid_levels, vec![(100.0, 3.0)]);
        assert_eq!(engine.book_ask_levels, vec![(100.2, 1.0)]);
        assert_eq!(
            engine.book_history.back().map(|sample| sample.timestamp),
            Some(20_000)
        );
        assert_eq!(engine.book_input_provenance.received, 3);
        assert_eq!(engine.book_input_provenance.accepted, 1);
        assert_eq!(engine.book_input_provenance.equal_timestamp_rejected, 1);
        assert_eq!(engine.book_input_provenance.older_timestamp_rejected, 1);
    }

    #[test]
    fn book_input_diagnostics_capture_source_time_receive_time_and_order_disposition() {
        let mut engine = SignalEngine::default();
        engine.on_book_with_receive_time(20_000, 30_000, 100.0, 100.2, &[1.0], &[1.0], &[], &[]);
        engine.on_book_with_receive_time(10_000, 31_000, 90.0, 90.2, &[1.0], &[1.0], &[], &[]);

        let diagnostic = engine.input_diagnostic(32_000);
        assert!(diagnostic.contains("book_events_received=2"));
        assert!(diagnostic.contains("book_quotes_accepted=1"));
        assert!(diagnostic.contains("book_older_rejected=1"));
        assert!(diagnostic.contains("book_last_source_timestamp_ms=Some(10000)"));
        assert!(diagnostic.contains("book_last_received_at_ms=Some(31000)"));
        assert!(diagnostic.contains("book_last_source_receive_delta_ms=Some(21000)"));
        assert!(diagnostic.contains("book_last_accepted_source_timestamp_ms=Some(20000)"));
    }

    #[test]
    fn scout_creates_independent_fee_aware_horizon_observations() {
        let mut engine = SignalEngine::default();
        engine.on_book(
            1_000,
            99.9,
            100.1,
            &[1.0],
            &[1.0],
            &[(99.9, 1.0)],
            &[(100.1, 1.0)],
        );
        engine.on_price(1_000, 100.0, 0.0);
        assert!(engine.track_scout(
            "sample-1".into(),
            "BTC-USDT".into(),
            "live-momentum".into(),
            1_000,
            "bullish".into(),
            100.0,
            0.5,
            0.8,
            0.15,
            0.1,
            0.05,
            None,
            None,
            None,
            None,
        ));
        assert_eq!(engine.pending.len(), 5);
        assert_eq!(
            engine
                .pending
                .iter()
                .filter(|pending| pending.scout_observation_only)
                .map(|pending| pending.horizon_seconds)
                .collect::<Vec<_>>(),
            vec![60, 300, 900, 3_600]
        );

        engine.on_book(
            61_000,
            100.1,
            100.3,
            &[1.0],
            &[1.0],
            &[(100.1, 1.0)],
            &[(100.3, 1.0)],
        );
        let outcomes = engine.on_price(61_000, 100.2, 0.0);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].scout_id.as_deref(), Some("sample-1"));
        assert!(outcomes[0].scout_observation_only);
        assert_eq!(outcomes[0].evaluation.horizon_seconds, 60);
        assert!(outcomes[0].evaluation.long_net_percent.is_some());
        assert!(!engine.completed_scout_ids.contains("sample-1"));
    }

    fn start_observation_scout(direction: &str, sample_id: &str) -> SignalEngine {
        let mut engine = SignalEngine::default();
        engine.on_book(1_000, 99.9, 100.1, &[1.0], &[1.0], &[], &[]);
        engine.on_price(1_000, 100.0, 0.0);
        assert!(engine.track_scout(
            sample_id.into(),
            "BTC-USDT".into(),
            "observation-test".into(),
            1_000,
            direction.into(),
            100.0,
            5.0,
            5.0,
            1.0,
            0.5,
            0.1,
            None,
            None,
            None,
            None,
        ));
        engine
    }

    #[test]
    fn replayed_scout_path_after_process_restart_is_never_complete() {
        let mut engine = start_observation_scout("bullish", "restart-case");
        assert!(engine.mark_scout_path_interrupted("restart-case"));
        for second in 2..=61 {
            engine.on_book(
                second * 1_000,
                100.0 + second as f64 * 0.01,
                100.2 + second as f64 * 0.01,
                &[1.0],
                &[1.0],
                &[],
                &[],
            );
        }
        let outcomes = engine.on_price(61_000, 100.61, 0.0);
        let path = outcomes
            .iter()
            .find(|outcome| outcome.scout_observation_only)
            .unwrap()
            .evaluation
            .scout_path_evidence
            .as_ref()
            .unwrap();
        assert!(!path.complete);
        assert_eq!(path.completeness_reason, "process_restart_interrupted");
    }

    #[test]
    fn restored_executable_samples_rebuild_excursions_but_keep_restart_gap_incomplete() {
        let mut before_restart = start_observation_scout("bullish", "durable-path");
        before_restart.set_scout_path_epoch("durable-path", 10);
        let mut durable_samples = Vec::new();
        for (timestamp, bid, ask) in [(2_000, 100.3, 100.5), (3_000, 99.9, 100.1)] {
            before_restart.on_book(timestamp, bid, ask, &[1.0], &[1.0], &[], &[]);
            durable_samples
                .extend(before_restart.latest_scout_path_samples(timestamp, bid, ask, 4));
        }

        let mut after_restart = start_observation_scout("bullish", "durable-path");
        after_restart.restore_scout_path_samples(
            "durable-path",
            &durable_samples,
            false,
            Some(4),
            2,
            Some((99.9, 100.1, 1_000)),
        );
        after_restart.set_scout_path_epoch("durable-path", 20);
        assert_eq!(after_restart.pending[0].scout_path.sample_count, 2);
        assert!(
            (after_restart.pending[0]
                .scout_path
                .maximum_favorable_excursion_percent
                .unwrap()
                - 0.1998)
                .abs()
                < 0.002
        );
        assert!(
            (after_restart.pending[0]
                .scout_path
                .maximum_adverse_excursion_percent
                .unwrap()
                + 0.1998)
                .abs()
                < 0.002
        );
        after_restart.mark_scout_path_interrupted("durable-path");
        after_restart.on_book(61_000, 100.0, 100.2, &[1.0], &[1.0], &[], &[]);
        let outcomes = after_restart.on_price(61_000, 100.1, 0.0);
        let evidence = outcomes
            .iter()
            .find(|outcome| outcome.scout_observation_only)
            .unwrap()
            .evaluation
            .scout_path_evidence
            .as_ref()
            .unwrap();
        assert_eq!(evidence.sample_count, 3);
        assert!(!evidence.complete);
        assert_eq!(evidence.completeness_reason, "process_restart_interrupted");
    }

    #[test]
    fn scout_path_retention_is_bounded_and_reports_truncation() {
        let mut engine = start_observation_scout("bullish", "bounded-path");
        for timestamp in 2_000..=6_200 {
            engine.on_book(timestamp, 100.0, 100.2, &[1.0], &[1.0], &[], &[]);
        }
        let path = &engine.pending[0].scout_path;
        assert_eq!(
            path.retained_samples.len(),
            SignalEngine::MAX_RETAINED_SCOUT_PATH_SAMPLES
        );
        assert!(path.truncated);
        assert!(path.sample_count > path.retained_samples.len() as u64);
        let outcomes = engine.on_price(61_000, 100.0, 0.0);
        let evidence = outcomes
            .iter()
            .find(|outcome| outcome.scout_observation_only)
            .unwrap()
            .evaluation
            .scout_path_evidence
            .as_ref()
            .unwrap();
        assert!(!evidence.complete);
        assert_eq!(evidence.completeness_reason, "retained_path_truncated");
    }

    #[test]
    fn duplicate_scout_registration_does_not_duplicate_path_state() {
        let mut engine = start_observation_scout("bullish", "idempotent-path");
        assert_eq!(engine.pending.len(), 5);
        assert!(!engine.track_scout(
            "idempotent-path".into(),
            "BTC-USDT".into(),
            "observation-test".into(),
            1_000,
            "bullish".into(),
            100.0,
            5.0,
            5.0,
            1.0,
            0.5,
            0.1,
            None,
            None,
            None,
            None,
        ));
        assert_eq!(engine.pending.len(), 5);
    }

    #[test]
    fn failed_durable_capture_forces_incomplete_evidence() {
        let mut engine = start_observation_scout("bullish", "capture-failed");
        assert!(engine.mark_scout_path_capture_failed("capture-failed"));
        for second in 2..=61 {
            engine.on_book(second * 1_000, 100.0, 100.2, &[1.0], &[1.0], &[], &[]);
        }
        let outcomes = engine.on_price(61_000, 100.0, 0.0);
        let evidence = outcomes
            .iter()
            .find(|outcome| outcome.scout_observation_only)
            .unwrap()
            .evaluation
            .scout_path_evidence
            .as_ref()
            .unwrap();
        assert!(!evidence.complete);
        assert_eq!(evidence.completeness_reason, "durable_capture_failed");
    }

    #[test]
    fn scout_path_uses_executable_bid_for_longs_and_ask_for_shorts() {
        for (direction, favorable_quote, adverse_quote, favorable, adverse, side) in [
            (
                "bullish",
                (100.3, 100.5),
                (99.9, 100.1),
                0.1998,
                -0.1998,
                "sell_bid_for_long",
            ),
            (
                "bearish",
                (99.5, 99.7),
                (99.9, 100.1),
                0.2002,
                -0.2002,
                "buy_ask_for_short",
            ),
        ] {
            let mut engine = start_observation_scout(direction, "path-case");
            for second in 2..=61 {
                let quote = if second == 2 {
                    favorable_quote
                } else if second == 3 {
                    adverse_quote
                } else {
                    (99.9, 100.1)
                };
                engine.on_book(second * 1_000, quote.0, quote.1, &[1.0], &[1.0], &[], &[]);
            }
            let after_horizon_quote = if direction == "bullish" {
                (101.0, 101.1)
            } else {
                (98.8, 99.0)
            };
            engine.on_book(
                62_000,
                after_horizon_quote.0,
                after_horizon_quote.1,
                &[1.0],
                &[1.0],
                &[],
                &[],
            );
            let outcomes = engine.on_price(62_000, 100.0, 0.0);
            let observation = outcomes
                .iter()
                .find(|outcome| outcome.scout_observation_only)
                .unwrap();
            let path = observation.evaluation.scout_path_evidence.as_ref().unwrap();
            assert_eq!(path.direction, direction);
            assert_eq!(path.executable_side, side);
            assert!(path.observation_only);
            assert_eq!(path.sample_count, 60);
            assert_eq!(path.last_source_event_timestamp, Some(61_000));
            assert_eq!(path.maximum_gap_ms, 1_000);
            assert!(path.source_timestamps_monotonic);
            assert!(path.complete, "{}", path.completeness_reason);
            assert!((path.maximum_favorable_excursion_percent.unwrap() - favorable).abs() < 0.002);
            assert!((path.maximum_adverse_excursion_percent.unwrap() - adverse).abs() < 0.002);
            assert!(outcomes.iter().all(|outcome| {
                outcome.scout_observation_only
                    == outcome
                        .evaluation
                        .scout_path_evidence
                        .as_ref()
                        .unwrap()
                        .observation_only
            }));
        }
    }

    #[test]
    fn scout_path_marks_out_of_order_and_wide_gaps_incomplete() {
        let mut engine = start_observation_scout("bullish", "ordering-case");
        engine.on_book(2_000, 100.0, 100.2, &[1.0], &[1.0], &[], &[]);
        engine.on_book(1_500, 100.1, 100.3, &[1.0], &[1.0], &[], &[]);
        engine.on_book(8_000, 100.0, 100.2, &[1.0], &[1.0], &[], &[]);
        engine.on_book(61_000, 100.0, 100.2, &[1.0], &[1.0], &[], &[]);
        let outcomes = engine.on_price(61_000, 100.1, 0.0);
        let path = outcomes
            .iter()
            .find(|outcome| outcome.scout_observation_only)
            .unwrap()
            .evaluation
            .scout_path_evidence
            .as_ref()
            .unwrap();
        assert!(!path.source_timestamps_monotonic);
        assert!(!path.complete);
        assert_eq!(path.completeness_reason, "non_monotonic_source_timestamps");
        assert!(path.maximum_gap_ms > MAX_BOOK_AGE_MS);
    }

    #[test]
    fn scout_path_requires_fresh_source_coverage_at_observation_horizon() {
        let mut engine = start_observation_scout("bullish", "stale-case");
        engine.on_book(2_000, 100.0, 100.2, &[1.0], &[1.0], &[], &[]);
        let outcomes = engine.on_price(61_000, 100.0, 0.0);
        let path = outcomes
            .iter()
            .find(|outcome| outcome.scout_observation_only)
            .unwrap()
            .evaluation
            .scout_path_evidence
            .as_ref()
            .unwrap();
        assert!(!path.complete);
        assert_eq!(path.completeness_reason, "stale_at_evaluation");
        assert_eq!(path.source_freshness_ms, Some(59_000));
    }

    #[test]
    fn scout_path_without_authoritative_entry_quote_does_not_impute_excursions() {
        let mut engine = SignalEngine::default();
        engine.on_price(1_000, 100.0, 0.0);
        assert!(engine.track_scout(
            "no-entry-book".into(),
            "BTC-USDT".into(),
            "observation-test".into(),
            1_000,
            "bullish".into(),
            100.0,
            5.0,
            5.0,
            1.0,
            0.5,
            0.1,
            None,
            None,
            None,
            None,
        ));
        for second in 2..=61 {
            engine.on_book(second * 1_000, 99.9, 100.1, &[1.0], &[1.0], &[], &[]);
        }
        let outcomes = engine.on_price(61_000, 100.0, 0.0);
        let path = outcomes
            .iter()
            .find(|outcome| outcome.scout_observation_only)
            .unwrap()
            .evaluation
            .scout_path_evidence
            .as_ref()
            .unwrap();
        assert_eq!(path.maximum_favorable_excursion_percent, None);
        assert_eq!(path.maximum_adverse_excursion_percent, None);
        assert_eq!(path.completeness_reason, "missing_entry_quote");
        assert!(!path.complete);
    }

    #[test]
    fn rejected_late_observation_only_outcome_increments_its_horizon_metric() {
        let mut engine = SignalEngine::default();
        engine.on_book(
            1_000,
            99.9,
            100.1,
            &[1.0],
            &[1.0],
            &[(99.9, 1.0)],
            &[(100.1, 1.0)],
        );
        engine.on_price(1_000, 100.0, 0.0);
        assert!(engine.track_scout(
            "late-sample".into(),
            "BTC-USDT".into(),
            "live-momentum".into(),
            1_000,
            "bullish".into(),
            100.0,
            0.5,
            0.8,
            0.15,
            0.1,
            0.05,
            None,
            None,
            None,
            None,
        ));
        // Isolate the 1h observation so the metric's horizon label is tested
        // independently of the shorter observations from this same scout.
        engine
            .pending
            .retain(|pending| pending.scout_observation_only && pending.horizon_seconds == 3_600);

        let before = crate::latency::snapshot();
        let before_count =
            before["late_scout_observation_only_drops_by_horizon_seconds"]["3600"]["dropped"]
                .as_u64()
                .unwrap();
        let before_total_lateness = before["late_scout_observation_only_drops_by_horizon_seconds"]
            ["3600"]["total_lateness_ms"]
            .as_u64()
            .unwrap();

        // The 1h observation is rejected just beyond its one-horizon lateness
        // allowance. No outcome may be emitted or relabeled as a 2h result.
        let outcomes = engine.on_price(7_201_001, 101.0, 0.0);
        assert!(outcomes.is_empty());

        let after = crate::latency::snapshot();
        let one_hour_metric =
            &after["late_scout_observation_only_drops_by_horizon_seconds"]["3600"];
        assert!(one_hour_metric["dropped"].as_u64().unwrap() >= before_count + 1);
        assert!(
            one_hour_metric["total_lateness_ms"].as_u64().unwrap()
                >= before_total_lateness + 3_600_001
        );
        assert!(one_hour_metric["max_lateness_ms"].as_u64().unwrap() >= 3_600_001);
    }

    #[test]
    fn counterfactual_registration_creates_fixed_horizons_and_deduplicates() {
        let mut engine = SignalEngine::default();
        assert!(engine.track_counterfactual("BTC-USDT".into(), 1_000, "bullish".into(), 100.0,));
        assert!(!engine.track_counterfactual("BTC-USDT".into(), 1_000, "bullish".into(), 100.0,));
        assert_eq!(engine.pending.len(), 4);
        assert_eq!(
            engine
                .pending
                .iter()
                .map(|pending| pending.horizon_seconds)
                .collect::<Vec<_>>(),
            vec![60, 300, 900, 3_600]
        );
    }

    #[test]
    fn book_history_is_sampled_and_bounded_under_high_frequency_updates() {
        let mut engine = SignalEngine::default();
        for timestamp in 0..120_000 {
            engine.on_book(
                timestamp,
                99.9,
                100.1,
                &[1.0],
                &[1.0],
                &[(99.9, 1.0)],
                &[(100.1, 1.0)],
            );
        }
        assert!(engine.book_history.len() <= BOOK_HISTORY_SAMPLE_LIMIT);
        assert!(engine.book_history.len() <= 601);
        assert_eq!(engine.book_history.back().unwrap().timestamp, 119_900);
    }

    #[test]
    fn scout_uses_stop_target_and_strategy_invalidation_instead_of_one_hour() {
        let mut engine = SignalEngine::default();
        engine.on_book(
            1_000,
            99.9,
            100.1,
            &[1.0],
            &[1.0],
            &[(99.9, 1.0)],
            &[(100.1, 1.0)],
        );
        engine.on_price(1_000, 100.0, 0.0);
        assert!(engine.track_scout(
            "scout-1".into(),
            "BTC-USDT".into(),
            "donchian-20-btc200".into(),
            1_000,
            "bullish".into(),
            100.0,
            0.5,
            0.8,
            0.15,
            0.1,
            0.05,
            None,
            None,
            None,
            None,
        ));
        engine.on_book(
            3_602_000,
            100.0,
            100.2,
            &[1.0],
            &[1.0],
            &[(100.0, 1.0)],
            &[(100.2, 1.0)],
        );
        let observations = engine.on_price(3_602_000, 100.1, 0.0);
        // The first price after the 1h boundary is much too late to represent
        // the 1m/5m/15m horizons. Keep only the 1h observation instead of
        // fabricating three stale horizon measurements.
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].evaluation.horizon_seconds, 3_600);
        assert!(
            observations
                .iter()
                .all(|outcome| outcome.scout_observation_only)
        );

        let active = BTreeSet::from([("donchian-20-btc200".into(), "BTC-USDT".into())]);
        let enabled = BTreeSet::from(["donchian-20-btc200".into()]);
        engine.update_scout_strategy_state(&active, &BTreeSet::new(), &enabled);
        engine.update_scout_strategy_state(&BTreeSet::new(), &BTreeSet::new(), &enabled);
        engine.on_book(
            3_603_000,
            100.0,
            100.2,
            &[1.0],
            &[1.0],
            &[(100.0, 1.0)],
            &[(100.2, 1.0)],
        );
        let outcomes = engine.on_price(3_603_000, 100.1, 0.0);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].evaluation.horizon_seconds, 3_602);
        assert_eq!(outcomes[0].evaluation.signal_timestamp, 1_000);
        assert!(!engine.track_scout(
            "scout-1".into(),
            "BTC-USDT".into(),
            "donchian-20-btc200".into(),
            1_000,
            "bullish".into(),
            100.0,
            0.5,
            0.8,
            0.15,
            0.1,
            0.05,
            None,
            None,
            None,
            None,
        ));
    }

    #[test]
    fn parameter_probe_exit_state_matches_the_exact_profile_variant() {
        let mut engine = SignalEngine::default();
        assert!(engine.track_scout(
            "probe-1".into(),
            "ETH-USDT".into(),
            "rsi2-oversold-rebound".into(),
            1_000,
            "bullish".into(),
            100.0,
            0.5,
            0.8,
            0.15,
            0.1,
            0.05,
            Some("rsi_exit_level".into()),
            Some(65.0),
            None,
            Some(5.0),
        ));
        let base_active = BTreeSet::from([("rsi2-oversold-rebound".into(), "ETH-USDT".into())]);
        let enabled = BTreeSet::from(["rsi2-oversold-rebound".into()]);
        engine.update_scout_strategy_state(&base_active, &BTreeSet::new(), &enabled);
        assert!(!engine.pending[0].strategy_active_seen);
        let active_variant = BTreeSet::from([(
            "rsi2-oversold-rebound".into(),
            "ETH-USDT".into(),
            "rsi_exit_level".into(),
            "65.00000000".into(),
        )]);
        engine.update_scout_strategy_state(&BTreeSet::new(), &active_variant, &enabled);
        assert!(engine.pending[0].strategy_active_seen);
        engine.update_scout_strategy_state(&BTreeSet::new(), &BTreeSet::new(), &enabled);
        assert!(engine.pending[0].strategy_exit_requested);
    }

    #[test]
    fn probe_hold_parameters_control_simulated_exit_timing() {
        let mut engine = SignalEngine::default();
        engine.on_book(
            1_000,
            99.9,
            100.1,
            &[1.0],
            &[1.0],
            &[(99.9, 1.0)],
            &[(100.1, 1.0)],
        );
        engine.on_price(1_000, 100.0, 0.0);
        assert!(engine.track_scout(
            "minimum-hold".into(),
            "BTC-USDT".into(),
            "donchian-20-btc200-hold3".into(),
            1_000,
            "bullish".into(),
            100.0,
            0.5,
            0.8,
            0.15,
            0.1,
            0.05,
            Some("minimum_hold_days".into()),
            Some(1.0),
            Some(1.0),
            None,
        ));
        let enabled = BTreeSet::from(["donchian-20-btc200-hold3".into()]);
        let active_variant = BTreeSet::from([(
            "donchian-20-btc200-hold3".into(),
            "BTC-USDT".into(),
            "minimum_hold_days".into(),
            "1.00000000".into(),
        )]);
        engine.update_scout_strategy_state(&BTreeSet::new(), &active_variant, &enabled);
        engine.update_scout_strategy_state(&BTreeSet::new(), &BTreeSet::new(), &enabled);

        let half_day = 43_201_000;
        engine.on_book(
            half_day,
            100.0,
            100.2,
            &[1.0],
            &[1.0],
            &[(100.0, 1.0)],
            &[(100.2, 1.0)],
        );
        let horizon_observations = engine.on_price(half_day, 100.1, 0.0);
        assert!(
            horizon_observations
                .iter()
                .all(|outcome| outcome.scout_observation_only)
        );
        let one_day = 86_401_000;
        engine.on_book(
            one_day,
            100.0,
            100.2,
            &[1.0],
            &[1.0],
            &[(100.0, 1.0)],
            &[(100.2, 1.0)],
        );
        let outcomes = engine.on_price(one_day, 100.1, 0.0);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| !outcome.scout_observation_only)
                .count(),
            1
        );

        let mut maximum_hold = SignalEngine::default();
        maximum_hold.on_book(
            1_000,
            99.9,
            100.1,
            &[1.0],
            &[1.0],
            &[(99.9, 1.0)],
            &[(100.1, 1.0)],
        );
        maximum_hold.on_price(1_000, 100.0, 0.0);
        assert!(maximum_hold.track_scout(
            "maximum-hold".into(),
            "ETH-USDT".into(),
            "rsi2-oversold-rebound".into(),
            1_000,
            "bullish".into(),
            100.0,
            0.5,
            0.8,
            0.15,
            0.1,
            0.05,
            Some("maximum_hold_days".into()),
            Some(1.0),
            None,
            Some(1.0),
        ));
        maximum_hold.on_book(
            one_day,
            100.0,
            100.2,
            &[1.0],
            &[1.0],
            &[(100.0, 1.0)],
            &[(100.2, 1.0)],
        );
        let maximum_hold_outcomes = maximum_hold.on_price(one_day, 100.1, 0.0);
        assert!(maximum_hold_outcomes.iter().any(|outcome| {
            !outcome.scout_observation_only && outcome.scout_exit_reason == Some("maximum_hold")
        }));
    }

    #[test]
    fn live_scout_stop_and_target_exits_use_trade_price_when_book_quote_is_missing() {
        let mut engine = SignalEngine::default();
        assert!(engine.track_scout(
            "live-flow-target".into(),
            "ETH-USDT".into(),
            "live-trade-flow".into(),
            1_000,
            "bullish".into(),
            100.0,
            0.5,
            0.8,
            0.15,
            0.1,
            0.05,
            None,
            None,
            None,
            None,
        ));

        let outcomes = engine.on_price(2_000, 101.2, 0.0);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].strategy.as_deref(), Some("live-trade-flow"));
        assert_eq!(outcomes[0].scout_exit_reason, Some("stop_or_target"));
        assert!(outcomes[0].evaluation.long_net_percent.is_some());
    }

    #[test]
    fn tight_stop_target_scenario_exits_before_standard_scenario() {
        let mut pending = PendingOutcome {
            symbol: "BTC-USDT".into(),
            timestamp: 1_000,
            base_price: 100.0,
            direction: "bullish".into(),
            strength: 1.0,
            horizon_seconds: 0,
            entry_bid: Some(99.9),
            entry_ask: Some(100.1),
            trend_5m_volatility_units: None,
            features: None,
            score: 0.0,
            ranging_probability: None,
            entry_quote_timestamp: Some(1_000),
            scout_path: ScoutPathAccumulator {
                source_timestamps_monotonic: true,
                ..Default::default()
            },
            taker_fee_rate: 0.001,
            first_long_positive_net_timestamp: None,
            first_short_positive_net_timestamp: None,
            sample_id: Some("tight".into()),
            strategy: Some("donchian-20-btc200".into()),
            scout_parameter_key: None,
            scout_parameter_value: None,
            scout_minimum_hold_days: None,
            scout_maximum_hold_days: None,
            scout_maximum_duration_ms: None,
            scout_observation_only: false,
            stop_loss_percent: Some(0.25),
            take_profit_percent: Some(0.4),
            profit_protection_activation_percent: Some(0.15),
            profit_protection_giveback_percent: Some(0.1),
            profit_protection_minimum_lock_percent: Some(0.05),
            profit_protection_peak_return_percent: None,
            strategy_active_seen: false,
            strategy_exit_requested: false,
        };
        assert!(scout_stop_or_target_hit(
            &mut pending,
            Some((2_000, 101.0, 101.2))
        ));
        let mut standard = PendingOutcome {
            take_profit_percent: Some(0.8),
            ..pending
        };
        assert!(!scout_stop_or_target_hit(
            &mut standard,
            Some((2_000, 101.0, 101.2))
        ));
    }
}
