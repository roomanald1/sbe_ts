impl SignalEngine {
    pub fn snapshot(&mut self, symbol: &str, now: u64) -> Option<SignalSnapshot> {
        self.prune(now);
        let latest = self.prices.back().copied()?;
        let range_cutoff = now.saturating_sub(300_000);
        let mut range_count = 0usize;
        let mut range_first_price = None;
        let mut range_low = f64::INFINITY;
        let mut range_high = f64::NEG_INFINITY;
        for &(timestamp, price) in &self.prices {
            if timestamp < range_cutoff {
                continue;
            }
            range_count += 1;
            range_first_price.get_or_insert(price);
            range_low = range_low.min(price);
            range_high = range_high.max(price);
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
            for &(timestamp, price) in &self.prices {
                if timestamp < range_cutoff {
                    continue;
                }
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
        let volatility_5m = self.realized_volatility(now.saturating_sub(300_000));
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
            if is_buy {
                buy += notional;
            } else {
                sell += notional;
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
            .filter(|t| t.timestamp >= now.saturating_sub(ABSORPTION_WINDOW_MS))
        {
            let (touch, prior_depth, bid_side) = if trade.is_buy {
                (trade.ask, trade.ask_depth, false)
            } else {
                (trade.bid, trade.bid_depth, true)
            };
            if prior_depth <= 0.0 || (trade.price - touch).abs() > touch * 2.0 / 10_000.0 {
                continue;
            }
            let replenished = self.book_history.iter().any(|book| {
                let (level, levels) = if bid_side {
                    (book.bid, &book.bid_levels)
                } else {
                    (book.ask, &book.ask_levels)
                };
                book.timestamp >= trade.timestamp
                    && book.timestamp <= trade.timestamp.saturating_add(2_000)
                    && (level - touch).abs() <= touch * 1e-8
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
        let components = SignalComponents {
            trend_1m_percent: trend,
            momentum_10s_percent: momentum,
            trade_flow_imbalance: flow,
            book_imbalance: self.book_imbalance,
            volatility_1m_percent: self
                .volatility_1m
                .or_else(|| self.realized_volatility(now.saturating_sub(60_000))),
            trend_5m_percent: trend_5m,
            trend_5m_volatility_units,
            mispricing_vs_vwap_bps,
            pressure_without_price_move,
            aggressor_cost_bps,
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
        let breakout_cutoff = now.saturating_sub(10_000);
        let mut prior_low = f64::INFINITY;
        let mut prior_high = f64::NEG_INFINITY;
        let mut prior_count = 0usize;
        for &(timestamp, price) in &self.prices {
            if timestamp >= range_cutoff && timestamp < breakout_cutoff {
                prior_low = prior_low.min(price);
                prior_high = prior_high.max(price);
                prior_count += 1;
            }
        }
        let breakout_clearance_percent = self
            .volatility_1m
            .or_else(|| self.realized_volatility(now.saturating_sub(60_000)))
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
        let reasons = snapshot_reasons(SnapshotReasonInputs {
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
        });
        Some(SignalSnapshot {
            symbol: symbol.to_owned(),
            timestamp: now,
            direction: direction.to_owned(),
            score,
            strength: (score.abs() * count as f64 / 9.0).clamp(0.0, 1.0),
            components,
            reasons,
            stale: now.saturating_sub(latest.0) > 5_000,
            ranging_probability,
            range_lower_bound: range_bounds.map(|(low, _)| low),
            range_upper_bound: range_bounds.map(|(_, high)| high),
        })
    }
}
