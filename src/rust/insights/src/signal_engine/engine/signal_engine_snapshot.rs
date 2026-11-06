impl SignalEngine {
    pub fn snapshot(&mut self, symbol: &str, now: u64) -> Option<SignalSnapshot> {
        self.snapshot_with_reasons(symbol, now, true)
    }

    pub fn snapshot_with_reasons(
        &mut self,
        symbol: &str,
        now: u64,
        include_reasons: bool,
    ) -> Option<SignalSnapshot> {
        self.prune(now);
        let latest = self.prices.back().copied()?;
        let price_count = self.prices.len();
        let mut price_change_square_prefix = vec![0.0; price_count + 1];
        for index in 1..price_count {
            let previous = self.prices[index - 1].1;
            let current = self.prices[index].1;
            let change = (current / previous).ln();
            price_change_square_prefix[index + 1] = price_change_square_prefix[index] + change * change;
        }
        let trade_count = self.trades.len();
        let (mut trade_buy_prefix, mut trade_sell_prefix) = (vec![0.0; trade_count + 1], vec![0.0; trade_count + 1]);
        let (mut trade_notional_prefix, mut trade_quantity_prefix) = (vec![0.0; trade_count + 1], vec![0.0; trade_count + 1]);
        for (index, &(timestamp, price, size, is_buy, _)) in self.trades.iter().enumerate() {
            let notional = price * size;
            trade_buy_prefix[index + 1] = trade_buy_prefix[index] + if is_buy { notional } else { 0.0 };
            trade_sell_prefix[index + 1] = trade_sell_prefix[index] + if is_buy { 0.0 } else { notional };
            trade_notional_prefix[index + 1] = trade_notional_prefix[index] + notional;
            trade_quantity_prefix[index + 1] = trade_quantity_prefix[index] + size;
            let _ = timestamp;
        }
        let range_cutoff = now.saturating_sub(300_000);
        let range_start = self
            .prices
            .partition_point(|sample| sample.0 < range_cutoff);
        let breakout_cutoff = now.saturating_sub(10_000);
        let breakout_end = self
            .prices
            .partition_point(|sample| sample.0 < breakout_cutoff);
        let mut range_count = 0usize;
        let mut range_first_price = None;
        let mut range_low = f64::INFINITY;
        let mut range_high = f64::NEG_INFINITY;
        let mut prior_low = f64::INFINITY;
        let mut prior_high = f64::NEG_INFINITY;
        let mut prior_count = 0usize;
        let mut previous_price: Option<f64> = None;
        let mut volatility_sum = 0.0_f64;
        for (offset, &(_, price)) in self.prices.range(range_start..).enumerate() {
            let index = range_start + offset;
            range_count += 1;
            range_first_price.get_or_insert(price);
            range_low = range_low.min(price);
            range_high = range_high.max(price);
            if let Some(previous) = previous_price {
                let change = (price / previous).ln();
                volatility_sum += change * change;
            }
            previous_price = Some(price);
            if index < breakout_end {
                prior_low = prior_low.min(price);
                prior_high = prior_high.max(price);
                prior_count += 1;
            }
        }
        let range_bounds = if range_count >= 12
            && range_low.is_finite()
            && range_high.is_finite()
            && range_low > 0.0
            && range_high > range_low
        {
            Some((range_low, range_high))
        } else {
            None
        };
        let ranging_probability = range_bounds.map(|(low, high)| {
            let width_percent = (high / low - 1.0) * 100.0;
            let net_move_percent =
                (latest.1 / range_first_price.unwrap_or(latest.1) - 1.0).abs() * 100.0;
            // Heuristic only: range likelihood rises when price explores both
            // edges but makes little net progress over the five-minute window.
            let mut low_touches = 0usize;
            let mut high_touches = 0usize;
            for &(_, price) in self.prices.range(range_start..) {
                if (price / low - 1.0) * 100.0 <= width_percent * 0.15 {
                    low_touches += 1;
                }
                if (high / price - 1.0) * 100.0 <= width_percent * 0.15 {
                    high_touches += 1;
                }
            }
            let exploration =
                ((low_touches.min(3) + high_touches.min(3)) as f64 / 6.0).clamp(0.0, 1.0);
            let containment =
                (1.0 - net_move_percent / (width_percent * 0.6).max(0.01)).clamp(0.0, 1.0);
            (0.65 * containment + 0.35 * exploration).clamp(0.0, 1.0)
        });
        let trend = self.price_return(now.saturating_sub(60_000));
        let momentum = self.price_return(now.saturating_sub(10_000));
        let trend_5m = self.price_return(now.saturating_sub(300_000));
        let volatility_5m = (range_count >= 2).then_some(volatility_sum.sqrt() * 100.0);
        let volatility_1m = self
            .volatility_1m
            .or_else(|| self.realized_volatility(now.saturating_sub(60_000)));
        let trend_5m_volatility_units = trend_5m
            .zip(volatility_5m)
            .and_then(|(trend, volatility)| (volatility > 1e-9).then_some(trend / volatility));
        let trade_cutoff = now.saturating_sub(60_000);
        let (mut buy, mut sell) = (0.0, 0.0);
        let (mut recent_notional, mut recent_quantity) = (0.0, 0.0);
        let (mut recent_count, mut recent_first, mut recent_last) = (0usize, None, None);
        let (mut weighted_cost, mut cost_notional) = (0.0, 0.0);
        for &(timestamp, price, size, is_buy, aggressor_cost) in &self.trades {
            let notional = price * size;
            if timestamp >= now.saturating_sub(60_000) {
                if is_buy {
                    buy += notional;
                } else {
                    sell += notional;
                }
            }
            if timestamp >= trade_cutoff {
                recent_count += 1;
                recent_first.get_or_insert(timestamp);
                recent_last = Some(timestamp);
                recent_notional += notional;
                recent_quantity += size;
                if let Some(cost) = aggressor_cost {
                    weighted_cost += cost * notional;
                    cost_notional += notional;
                }
            }
        }
        let flow = (buy + sell > 0.0).then_some((buy - sell) / (buy + sell));
        let mispricing_vs_vwap_bps = if recent_count >= 5
            && recent_last
                .zip(recent_first)
                .is_some_and(|(last, first)| last.saturating_sub(first) >= 10_000)
            && recent_quantity > 0.0
        {
            let vwap = recent_notional / recent_quantity;
            (vwap.is_finite() && vwap > 0.0).then_some((latest.1 / vwap - 1.0) * 10_000.0)
        } else {
            None
        };
        let flow_without_move = flow.zip(momentum).and_then(|(imbalance, movement)| {
            (imbalance.abs() >= 0.55 && movement.abs() <= 0.025).then_some(-imbalance)
        });
        let aggressor_cost_bps = (cost_notional > 0.0).then_some(weighted_cost / cost_notional);
        // Count only market sells hitting the bid or buys lifting the ask when
        // the displayed quantity at that exact level later replenishes.
        let (mut absorbed_sell, mut absorbed_buy) = (0.0, 0.0);
        for trade in self
            .absorption_trades
            .iter()
            .filter(|t| t.timestamp >= now.saturating_sub(30_000))
        {
            let (touch, prior_depth, bid_side) = if trade.is_buy {
                (trade.ask, trade.ask_depth, false)
            } else {
                (trade.bid, trade.bid_depth, true)
            };
            if prior_depth <= 0.0 || (trade.price - touch).abs() > touch * 2.0 / 10_000.0 {
                continue;
            }
            // Book history is timestamp ordered. Narrow the search to the two
            // seconds after this trade instead of rescanning the full 60-second
            // history for every trade on every snapshot.
            let book_end_timestamp = trade.timestamp.saturating_add(2_000);
            let first_book = self
                .book_history
                .partition_point(|book| book.timestamp < trade.timestamp);
            let replenished = self
                .book_history
                .range(first_book..)
                .take_while(|book| book.timestamp <= book_end_timestamp)
                .any(|book| {
                    let (level, levels) = if bid_side {
                        (book.bid, &book.bid_levels)
                    } else {
                        (book.ask, &book.ask_levels)
                    };
                    (level - touch).abs() <= touch * 1e-8
                        && levels
                            .iter()
                            .find(|(px, _)| (*px - touch).abs() <= touch * 1e-8)
                            .is_some_and(|(_, qty)| *qty >= prior_depth + trade.size * 0.5)
                });
            if replenished {
                if bid_side {
                    absorbed_sell += trade.price * trade.size;
                } else {
                    absorbed_buy += trade.price * trade.size;
                }
            }
        }
        let absorption_imbalance = if momentum.is_some_and(|m| m.abs() <= 0.025)
            && absorbed_sell + absorbed_buy > 0.0
        {
            Some(((absorbed_sell - absorbed_buy) / (absorbed_sell + absorbed_buy)).clamp(-1.0, 1.0))
        } else {
            None
        };
        // Reuse the existing serialized field to keep persisted bincode records compatible.
        let pressure_without_price_move = absorption_imbalance.or(flow_without_move);
        let momentum_by_window = (5..=30)
            .step_by(5)
            .map(|window_seconds| WindowedSignalValue {
                window_seconds,
                value: self.price_return(now.saturating_sub(u64::from(window_seconds) * 1_000)),
                sample_count: 0,
                span_ms: 0,
            })
            .collect::<Vec<_>>();
        let trend_volatility_units_by_window = (60..=900)
            .step_by(60)
            .map(|window_seconds| {
                let cutoff = now.saturating_sub(u64::from(window_seconds) * 1_000);
                let start = self.prices.partition_point(|sample| sample.0 < cutoff);
                let sample_count = price_count.saturating_sub(start);
                let volatility = (sample_count >= 2).then(|| {
                    (price_change_square_prefix[price_count] - price_change_square_prefix[start + 1]).sqrt() * 100.0
                });
                let value = self.price_return(cutoff)
                    .zip(volatility)
                    .and_then(|(trend, volatility)| (volatility > 1e-9).then_some(trend / volatility));
                WindowedSignalValue {
                    window_seconds,
                    value,
                    sample_count: 0,
                    span_ms: 0,
                }
            })
            .collect::<Vec<_>>();
        let trade_flow_by_window = (10..=120)
            .step_by(10)
            .map(|window_seconds| {
                let cutoff = now.saturating_sub(u64::from(window_seconds) * 1_000);
                let start = self.trades.partition_point(|sample| sample.0 < cutoff);
                let buy = trade_buy_prefix[trade_count] - trade_buy_prefix[start];
                let sell = trade_sell_prefix[trade_count] - trade_sell_prefix[start];
                let first = self.trades.get(start).map(|sample| sample.0);
                let last = self.trades.back().map(|sample| sample.0);
                WindowedSignalValue {
                    window_seconds,
                    value: (buy + sell > 0.0).then_some((buy - sell) / (buy + sell)),
                    sample_count: trade_count.saturating_sub(start).min(u16::MAX as usize) as u16,
                    span_ms: last.zip(first).map_or(0, |(last, first)| last.saturating_sub(first).min(u64::from(u32::MAX)) as u32),
                }
            })
            .collect::<Vec<_>>();
        let book_imbalance_by_depth = (1..=10)
            .map(|depth_levels| {
                let bid = self.book_bid_levels.iter().take(depth_levels).map(|(_, size)| size).sum::<f64>();
                let ask = self.book_ask_levels.iter().take(depth_levels).map(|(_, size)| size).sum::<f64>();
                DepthSignalValue {
                    depth_levels: depth_levels as u8,
                    value: (depth_levels <= self.book_bid_levels.len()
                        && depth_levels <= self.book_ask_levels.len()
                        && bid + ask > 0.0)
                        .then_some((bid - ask) / (bid + ask)),
                }
            })
            .collect::<Vec<_>>();
        let vwap_dislocation_by_window = (10..=300)
            .step_by(10)
            .map(|window_seconds| {
                let cutoff = now.saturating_sub(u64::from(window_seconds) * 1_000);
                let start = self.trades.partition_point(|sample| sample.0 < cutoff);
                let notional = trade_notional_prefix[trade_count] - trade_notional_prefix[start];
                let quantity = trade_quantity_prefix[trade_count] - trade_quantity_prefix[start];
                let first = self.trades.get(start).map(|sample| sample.0);
                let last = self.trades.back().map(|sample| sample.0);
                let span_ms = last.zip(first).map_or(0, |(last, first)| last.saturating_sub(first).min(u64::from(u32::MAX)) as u32);
                let vwap = (quantity > 0.0).then_some(notional / quantity);
                WindowedSignalValue {
                    window_seconds,
                    value: vwap.filter(|value| value.is_finite() && *value > 0.0)
                        .map(|value| (latest.1 / value - 1.0) * 10_000.0),
                    sample_count: trade_count.saturating_sub(start).min(u16::MAX as usize) as u16,
                    span_ms,
                }
            })
            .collect::<Vec<_>>();
        let mut absorption_events = Vec::new();
        for trade in self.absorption_trades.iter().filter(|trade| trade.timestamp >= now.saturating_sub(60_000)) {
            let (touch, prior_depth, bid_side) = if trade.is_buy {
                (trade.ask, trade.ask_depth, false)
            } else {
                (trade.bid, trade.bid_depth, true)
            };
            if prior_depth <= 0.0 || touch <= 0.0 { continue; }
            let touch_distance_bps = (trade.price - touch).abs() / touch * 10_000.0;
            let first_book = self.book_history.partition_point(|book| book.timestamp < trade.timestamp);
            let replenishment_delay_ms = self.book_history.range(first_book..)
                .take_while(|book| book.timestamp <= trade.timestamp.saturating_add(5_000))
                .find_map(|book| {
                    let (level, levels) = if bid_side { (book.bid, &book.bid_levels) } else { (book.ask, &book.ask_levels) };
                    ((level - touch).abs() <= touch * 1e-8
                        && levels.iter().find(|(price, _)| (*price - touch).abs() <= touch * 1e-8)
                            .is_some_and(|(_, size)| *size >= prior_depth + trade.size * 0.5))
                        .then_some(book.timestamp.saturating_sub(trade.timestamp).min(u64::from(u32::MAX)) as u32)
                });
            absorption_events.push((trade.timestamp, trade.price * trade.size, bid_side, touch_distance_bps, replenishment_delay_ms));
        }
        // Accumulate each qualifying event into all wider windows and looser
        // gates once, rather than rescanning every event for every combination.
        let mut absorbed_sell = vec![vec![vec![0.0; 5]; 5]; 6];
        let mut absorbed_buy = vec![vec![vec![0.0; 5]; 5]; 6];
        for (timestamp, notional, bid_side, touch_distance, replenishment_delay) in &absorption_events {
            let age_ms = now.saturating_sub(*timestamp);
            if age_ms > 60_000 { continue; }
            let minimum_window = age_ms.div_ceil(10_000).saturating_sub(1).min(5) as usize;
            let Some(delay_ms) = replenishment_delay else { continue; };
            if *touch_distance > 5.0 { continue; }
            let minimum_replenishment = delay_ms.div_ceil(1_000).saturating_sub(1).min(4) as usize;
            let minimum_tolerance = (touch_distance.ceil() as usize).saturating_sub(1).min(4);
            let totals = if *bid_side { &mut absorbed_sell } else { &mut absorbed_buy };
            for windows in totals.iter_mut().skip(minimum_window) {
                for replenishment in windows.iter_mut().skip(minimum_replenishment) {
                    for value in replenishment.iter_mut().skip(minimum_tolerance) {
                        *value += *notional;
                    }
                }
            }
        }
        let mut absorption_pressure_by_window = Vec::with_capacity(6 * 5 * 5);
        for (window_index, window_seconds) in (10..=60).step_by(10).enumerate() {
            for (replenishment_index, replenishment_window_ms) in (1_000..=5_000).step_by(1_000).enumerate() {
                for (touch_index, touch_tolerance_bps) in (1..=5).enumerate() {
                    let sell = absorbed_sell[window_index][replenishment_index][touch_index];
                    let buy = absorbed_buy[window_index][replenishment_index][touch_index];
                    let pressure = (sell + buy > 0.0).then_some((sell - buy) / (sell + buy));
                    absorption_pressure_by_window.push(AbsorptionPressureValue {
                        window_seconds,
                        replenishment_window_ms,
                        touch_tolerance_bps: f64::from(touch_tolerance_bps),
                        value: pressure,
                    });
                }
            }
        }
        let components = SignalComponents {
            trend_1m_percent: trend,
            momentum_10s_percent: momentum,
            trade_flow_imbalance: flow,
            book_imbalance: self.book_imbalance,
            volatility_1m_percent: volatility_1m,
            trend_5m_percent: trend_5m,
            trend_5m_volatility_units,
            mispricing_vs_vwap_bps,
            pressure_without_price_move,
            aggressor_cost_bps,
            momentum_by_window,
            trend_volatility_units_by_window,
            trade_flow_by_window,
            book_imbalance_by_depth,
            vwap_dislocation_by_window,
            absorption_pressure_by_window,
        };
        let mispricing_signal =
            mispricing_vs_vwap_bps.map(|value| (-value / 20.0).clamp(-1.0, 1.0));
        // In a confidently ranging market, price near a range edge is a
        // contrarian mean-reversion opportunity. Keep this deliberately
        // gated so the edge vote does not fade moves in a trending regime.
        let mean_reversion_signal =
            range_bounds
                .zip(ranging_probability)
                .and_then(|((low, high), probability)| {
                    if probability < 0.70 {
                        return None;
                    }
                    let width = high - low;
                    let location = (latest.1 - low) / width;
                    if location <= 0.12 {
                        Some(0.7)
                    } else if location >= 0.88 {
                        Some(-0.7)
                    } else {
                        None
                    }
                });
        // Treat book pressure as actionable only when recent aggressive trades
        // confirm the same side. This reduces signals from transient displayed
        // liquidity that is not being consumed or supported by trade flow.
        let order_book_pressure_signal = self.book_imbalance.zip(flow).and_then(|(book, flow)| {
            if book >= 0.60 && flow >= 0.25 {
                Some(0.8)
            } else if book <= -0.60 && flow <= -0.25 {
                Some(-0.8)
            } else {
                None
            }
        });
        // Compare the latest price with an earlier five-minute range that
        // excludes the most recent ten seconds, so a new breakout can actually
        // cross the boundary. Require both short-term momentum and trade flow
        // to confirm direction, plus a volatility-scaled minimum clearance.
        let breakout_clearance_percent = volatility_1m
            .map(|volatility| (volatility.abs() * 0.20).max(0.02))
            .unwrap_or(0.02);
        let volatility_breakout_signal = if prior_count >= 12 {
            momentum.zip(flow).and_then(|(move_percent, imbalance)| {
                if latest.1 > prior_high * (1.0 + breakout_clearance_percent / 100.0)
                    && move_percent > 0.0
                    && imbalance >= 0.30
                {
                    Some(0.8)
                } else if latest.1 < prior_low * (1.0 - breakout_clearance_percent / 100.0)
                    && move_percent < 0.0
                    && imbalance <= -0.30
                {
                    Some(-0.8)
                } else {
                    None
                }
            })
        } else {
            None
        };
        let inputs = [
            trend.map(|v| (v / 0.25).clamp(-1.0, 1.0)),
            momentum.map(|v| (v / 0.12).clamp(-1.0, 1.0)),
            flow,
            self.book_imbalance,
            mispricing_signal,
            pressure_without_price_move,
            mean_reversion_signal,
            order_book_pressure_signal,
            volatility_breakout_signal,
        ];
        let count = inputs.iter().filter(|v| v.is_some()).count();
        if count == 0 {
            return None;
        }
        let score = inputs.iter().flatten().sum::<f64>() / count as f64;
        let direction = if score > 0.2 {
            "bullish"
        } else if score < -0.2 {
            "bearish"
        } else if ranging_probability.is_some_and(|probability| probability >= 0.65) {
            "ranging"
        } else {
            "neutral"
        };
        let reasons = include_reasons
            .then(|| {
                snapshot_reasons(SnapshotReasonInputs {
                    trend,
                    momentum,
                    flow,
                    book_imbalance: self.book_imbalance,
                    order_book_pressure_signal,
                    volatility_breakout_signal,
                    mispricing_vs_vwap_bps,
                    range_bounds,
                    mean_reversion_signal,
                    price: latest.1,
                    absorption_imbalance,
                    flow_without_move,
                    aggressor_cost_bps,
                    direction,
                    components: &components,
                    ranging_probability,
                    prior_low,
                    prior_high,
                    breakout_clearance_percent,
                })
            })
            .unwrap_or_default();
        Some(SignalSnapshot {
            symbol: symbol.to_owned(),
            timestamp: now,
            direction: direction.to_owned(),
            score,
            strength: (score.abs() * count as f64 / 9.0).clamp(0.0, 1.0),
            evidence_coverage: Some(count as f64 / 9.0),
            estimated_accuracy: None,
            components,
            reasons,
            news_attribution: None,
            stale: now.saturating_sub(latest.0) > 5_000,
            ranging_probability,
            range_lower_bound: range_bounds.map(|(low, _)| low),
            range_upper_bound: range_bounds.map(|(_, high)| high),
        })
    }
}
