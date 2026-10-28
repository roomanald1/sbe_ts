impl SignalEngine {
    pub fn research_features(
        &self,
        symbol: &str,
        timestamp: u64,
        btc: (Option<f64>, Option<f64>),
        eth: (Option<f64>, Option<f64>),
    ) -> SignalResearchFeatures {
        let quote = self.best_quote.filter(|(quote_time, _, _)| {
            *quote_time <= timestamp && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
        });
        let current_book = self.book_history.iter().rev().find(|sample| {
            sample.timestamp <= timestamp
                && timestamp.saturating_sub(sample.timestamp) <= MAX_BOOK_AGE_MS
        });
        let depth = |sample: &BookHistorySample, bids: bool| {
            let levels = if bids {
                &sample.bid_levels
            } else {
                &sample.ask_levels
            };
            levels
                .iter()
                .filter(|(price, _)| {
                    ((*price / ((sample.bid + sample.ask) / 2.0) - 1.0).abs() * 10_000.0) <= 10.0
                })
                .map(|(price, size)| price * size)
                .sum::<f64>()
        };
        let (bid_notional, ask_notional) = current_book
            .map(|sample| (Some(depth(sample, true)), Some(depth(sample, false))))
            .unwrap_or((None, None));
        let imbalance = bid_notional
            .zip(ask_notional)
            .and_then(|(bid, ask)| (bid + ask > 0.0).then_some((bid - ask) / (bid + ask)));
        let imbalance_at = |cutoff: u64| {
            self.book_history
                .iter()
                .rev()
                .find(|sample| sample.timestamp <= cutoff)
                .and_then(|sample| {
                    let bid = depth(sample, true);
                    let ask = depth(sample, false);
                    (bid + ask > 0.0).then_some((bid - ask) / (bid + ask))
                })
        };
        let current_imbalance = current_book.and_then(|sample| {
            let bid = depth(sample, true);
            let ask = depth(sample, false);
            (bid + ask > 0.0).then_some((bid - ask) / (bid + ask))
        });
        let current_mid = quote.map(|(_, bid, ask)| (bid + ask) / 2.0);
        let top_sizes = current_book
            .and_then(|sample| Some((*sample.bid_levels.first()?, *sample.ask_levels.first()?)));
        let microprice = current_mid.zip(top_sizes).and_then(|(mid, (bid, ask))| {
            (bid.1 + ask.1 > 0.0)
                .then_some(((sample_microprice(bid.0, bid.1, ask.0, ask.1) / mid) - 1.0) * 10_000.0)
        });
        let spread = quote.and_then(|(_, bid, ask)| {
            ((bid + ask) > 0.0).then_some((ask - bid) / ((ask + bid) / 2.0) * 10_000.0)
        });
        let flow = |window_ms: u64| {
            let start = timestamp.saturating_sub(window_ms);
            let mut signed = 0.0;
            let mut total = 0.0;
            let mut count = 0u64;
            for (_, price, size, is_buy, _) in self
                .trades
                .iter()
                .filter(|(trade_time, ..)| *trade_time >= start && *trade_time <= timestamp)
            {
                let notional = price * size;
                total += notional;
                signed += if *is_buy { notional } else { -notional };
                count += 1;
            }
            (total > 0.0).then_some((signed / total, signed, total, count))
        };
        let flow5 = flow(5_000);
        let flow15 = flow(15_000);
        let flow60 = flow(60_000);
        let signed5 = flow5.map(|(_, signed, _, _)| signed);
        let signed15 = flow15.map(|(_, signed, _, _)| signed);
        let trade_count_5s = flow5.map_or(0, |(_, _, _, count)| count);
        let trade_count_60s = flow60.map_or(0, |(_, _, _, count)| count);
        let average_trade_notional_5s =
            flow5.and_then(|(_, _, total, count)| (count > 0).then_some(total / count as f64));
        let price_move_bps = self
            .price_return(timestamp.saturating_sub(5_000))
            .map(|value| value * 100.0);
        SignalResearchFeatures {
            symbol: symbol.to_owned(),
            timestamp,
            spread_bps: spread,
            microprice_offset_bps: microprice,
            bid_notional_10bps: bid_notional,
            ask_notional_10bps: ask_notional,
            depth_imbalance_10bps: imbalance,
            depth_imbalance_change_1s: current_imbalance
                .zip(imbalance_at(timestamp.saturating_sub(1_000)))
                .map(|(now, old)| now - old),
            depth_imbalance_change_5s: current_imbalance
                .zip(imbalance_at(timestamp.saturating_sub(5_000)))
                .map(|(now, old)| now - old),
            trade_flow_5s: flow5.map(|(value, _, _, _)| value),
            trade_flow_15s: flow15.map(|(value, _, _, _)| value),
            trade_flow_60s: flow60.map(|(value, _, _, _)| value),
            trade_flow_acceleration: flow5
                .map(|(value, _, _, _)| value)
                .zip(flow60.map(|(value, _, _, _)| value))
                .map(|(short, long)| short - long),
            signed_trade_notional_5s: signed5,
            signed_trade_notional_15s: signed15,
            trade_count_5s,
            trade_count_60s,
            average_trade_notional_5s,
            price_response_bps_per_1000usd_5s: price_move_bps
                .zip(signed5)
                .filter(|(_, signed)| signed.abs() > 1e-9)
                .map(|(move_bps, signed)| move_bps * 1_000.0 / signed.abs().max(1.0)),
            btc_return_1m_percent: btc.0,
            btc_return_5m_percent: btc.1,
            eth_return_1m_percent: eth.0,
            eth_return_5m_percent: eth.1,
        }
    }
}
