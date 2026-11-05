// Runtime-only indexes over the append-only fill ledger. Warmed at load;
// submissions update only newly appended fills while holding the ledger lock.
#[derive(Default)]
struct LedgerTradeIndex {
    trade_count: usize,
    by_parent: HashMap<String, Vec<usize>>,
    positions: HashMap<Option<String>, BTreeMap<String, Position>>,
    counts: HashMap<Option<String>, usize>,
    cash_by_quote: BTreeMap<String, f64>,
    fees_by_quote: BTreeMap<String, f64>,
}

impl LedgerTradeIndex {
    fn extend(&mut self, trades: &[PositionTrade]) {
        for (offset, trade) in trades[self.trade_count..].iter().enumerate() {
            if let Some(parent) = &trade.parent_order_id {
                self.by_parent.entry(parent.clone()).or_default()
                    .push(self.trade_count + offset);
            }
            let quote = trade.symbol.split_once('-').map(|(_, q)| q).unwrap_or_default();
            let notional = trade.quantity * trade.execution_price;
            let cash = if matches!(trade.side, TradeSide::Buy) { -notional } else { notional };
            let fee = signed_fee_cashflow(trade.fee);
            *self.cash_by_quote.entry(quote.to_owned()).or_default() += cash + fee;
            *self.fees_by_quote.entry(quote.to_owned()).or_default() -= fee;
            for source in [None, Some(trade.source.clone())] {
                *self.counts.entry(source.clone()).or_default() += 1;
                let position = self.positions.entry(source).or_default()
                    .entry(trade.symbol.clone()).or_insert(Position {
                        symbol: trade.symbol.clone(), quantity: 0.0,
                        average_entry_price: 0.0, realized_pnl: 0.0,
                        entry_fees: 0.0, fees_paid: 0.0,
                        estimated_exit_fee_rate: 0.0, last_trade_timestamp: trade.timestamp,
                    });
                apply_trade(position, trade);
                position.last_trade_timestamp = trade.timestamp;
            }
        }
        self.trade_count = trades.len();
    }
}

fn with_ledger_trade_index<T>(
    store: &PositionStore, ledger: &Ledger,
    read: impl FnOnce(&LedgerTradeIndex) -> T,
) -> T {
    let mut index = store.trade_index.lock().unwrap_or_else(|p| p.into_inner());
    if index.trade_count > ledger.trades.len() {
        // Detached UI snapshots may be older than the authoritative ledger.
        // Never rewind the live index or rebuild it while holding its mutex.
        drop(index);
        let mut old_index = LedgerTradeIndex::default();
        old_index.extend(&ledger.trades);
        return read(&old_index);
    }
    index.extend(&ledger.trades);
    read(&index)
}

fn indexed_position_summary(
    store: &PositionStore, ledger: &Ledger, source: Option<&str>,
) -> PositionsResponse {
    let source = source.map(str::to_owned);
    with_ledger_trade_index(store, ledger, |index| PositionsResponse {
        positions: index.positions.get(&source)
            .map(|positions| positions.values().cloned().collect()).unwrap_or_default(),
        trade_count: index.counts.get(&source).copied().unwrap_or_default(),
    })
}

#[cfg(test)]
mod trade_index_tests {
    use super::*;

    fn trade(id: &str, side: &str, quantity: f64, source: &str) -> PositionTrade {
        serde_json::from_value(serde_json::json!({
            "id": id, "symbol": "BTC-USDT", "side": side, "quantity": quantity,
            "execution_price": 100.0, "timestamp": 1, "source": source,
            "parent_order_id": id, "fee": -1.0, "fee_rate": 0.001
        })).unwrap()
    }

    #[test]
    fn incremental_index_matches_full_replay_and_does_not_double_count() {
        let mut fills = vec![trade("a", "buy", 2.0, "auto")];
        let mut index = LedgerTradeIndex::default();
        index.extend(&fills);
        fills.push(trade("b", "sell", 3.0, "auto"));
        fills.push(trade("c", "buy", 0.5, "manual"));
        index.extend(&fills);
        index.extend(&fills);
        for source in [None, Some("auto".to_owned()), Some("manual".to_owned())] {
            let expected = summarize_positions_from(fills.iter().filter(|t|
                source.as_deref().is_none_or(|s| t.source == s)));
            let actual = index.positions[&source].values().cloned().collect::<Vec<_>>();
            assert_eq!(serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected.positions).unwrap());
            assert_eq!(index.counts[&source], expected.trade_count);
        }
        assert_eq!(index.by_parent["b"], vec![1]);
        assert_eq!(index.cash_by_quote["USDT"], 47.0);
        assert_eq!(index.fees_by_quote["USDT"], 3.0);
    }
}
