async fn get_fee_rates() -> Json<FeeRatesResponse> {
    let rates = configured_fee_rates();
    Json(FeeRatesResponse {
        maker: rates.maker.abs(),
        taker: rates.taker.abs(),
    })
}
#[derive(Clone)]
struct AccountLedgerSnapshot {
    cash_delta: f64,
    fees_paid: f64,
    unknown_fee_fills: usize,
    equity_available: bool,
    positions: BTreeMap<String, Position>,
}

fn account_snapshot_with_rates(
    store: &PositionStore,
    ledger: &Ledger,
    rates: &BTreeMap<String, f64>,
) -> Result<AccountResponse, (StatusCode, String)> {
    let snapshot = with_ledger_trade_index(store, ledger, |index| {
        let mut cash_delta = 0.0;
        let mut fees_paid = 0.0;
        let mut equity_available = true;
        for (quote, cash) in &index.cash_by_quote {
            if let Some(rate) = rates.get(quote) {
                cash_delta += cash * rate;
                fees_paid += index.fees_by_quote.get(quote).copied().unwrap_or_default() * rate;
            } else {
                equity_available = false;
            }
        }
        let unknown_fee_fills = index.unknown_fee_fills_by_quote.values().sum();
        AccountLedgerSnapshot {
            cash_delta, fees_paid, unknown_fee_fills, equity_available,
            positions: index.positions.get(&None).cloned().unwrap_or_default(),
        }
    });
    let cash = ledger.initial_cash
        + ledger.topups.iter().map(|topup| topup.amount).sum::<f64>()
        + snapshot.cash_delta
        + ledger
            .dated_carry_positions
            .iter()
            .filter(|position| position.status == "open")
            .map(|position| {
                position.quantity * (position.future_entry_price - position.spot_entry_price)
                    - position.entry_fees
            })
            .sum::<f64>();
    let fees_paid = snapshot.fees_paid
        + ledger
            .dated_carry_positions
            .iter()
            .filter(|position| position.status == "open")
            .map(|position| position.entry_fees)
            .sum::<f64>();
    let mut equity_available = snapshot.equity_available;
    let positions = snapshot.positions.into_values().collect::<Vec<_>>();
    let marks = store.market_marks_snapshot();
    let now = now_ms().unwrap_or_default();
    let mut equity = cash;
    let mut realized_pnl = 0.0;
    let mut unrealized_pnl = 0.0;
    let mut estimated_exit_fees = 0.0;
    for position in &positions {
        let quote = position
            .symbol
            .split_once('-')
            .map(|(_, quote)| quote)
            .unwrap_or_default();
        let Some(rate) = rates.get(quote) else {
            if position.quantity.abs() > EPSILON || position.realized_pnl.abs() > EPSILON {
                equity_available = false;
            }
            continue;
        };
        // Keep realized P&L from fully closed symbols; they remain in the
        // ledger summary with zero quantity, but still contribute to account P&L.
        realized_pnl += position.realized_pnl * rate;
        if position.quantity.abs() <= EPSILON {
            continue;
        }
        let Some(mark) = marks.get(&position.symbol) else {
            equity_available = false;
            continue;
        };
        if now.saturating_sub(mark.updated_at_ms) > 5_000 {
            equity_available = false;
            continue;
        }
        let mark = if position.quantity > 0.0 {
            mark.bid
        } else {
            mark.ask
        };
        let Some(mark) = mark else {
            equity_available = false;
            continue;
        };
        let marked_value = position.quantity * mark * rate;
        equity += marked_value;
        let gross_unrealized = (mark - position.average_entry_price) * position.quantity;
        let close_fee = position.quantity.abs() * mark * position.estimated_exit_fee_rate;
        estimated_exit_fees += close_fee * rate;
        // entry_fees is a signed cash flow (negative for a fee); close_fee is
        // an estimated positive cost, so subtract only the latter.
        unrealized_pnl += (gross_unrealized + position.entry_fees - close_fee) * rate;
    }
    for position in ledger
        .dated_carry_positions
        .iter()
        .filter(|position| position.status == "open")
    {
        let Some(marked_pnl) = position.marked_net_pnl_usdt.filter(|pnl| pnl.is_finite()) else {
            equity_available = false;
            continue;
        };
        let Some(spot_mark) = position.spot_mark_bid.filter(|price| price.is_finite() && *price > 0.0) else {
            equity_available = false;
            continue;
        };
        let Some(future_mark) = position.future_mark_ask.filter(|price| price.is_finite() && *price > 0.0) else {
            equity_available = false;
            continue;
        };
        let rate = rates.get("USDT").copied().unwrap_or_else(|| {
            equity_available = false;
            0.0
        });
        if rate > 0.0 {
            let gross_mark = position.quantity
                * (spot_mark - position.spot_entry_price + position.future_entry_price - future_mark);
            equity += gross_mark * rate;
            unrealized_pnl += marked_pnl * rate;
        }
    }
    Ok(AccountResponse {
        currency: "USD",
        cash,
        equity,
        realized_pnl,
        unrealized_pnl,
        fees_paid,
        fees_complete: snapshot.unknown_fee_fills == 0,
        unknown_fee_fills: snapshot.unknown_fee_fills,
        estimated_exit_fees,
        equity_available,
        initial_cash: ledger.initial_cash,
        topups: ledger.topups.clone(),
        max_leverage: configured_account_leverage(ledger),
    })
}

#[cfg(test)]
mod account_snapshot_tests {
    use super::*;

    fn store() -> PositionStore {
        let (state_updates, _) = broadcast::channel(1);
        PositionStore {
            path: PathBuf::new(),
            ledger: Arc::new(Mutex::new(Ledger::default())),
            persistence_notify: Arc::new(Notify::new()),
            state_updates,
            persistence_timings_ms: Arc::new(StdMutex::new(BTreeMap::new())),
            trade_index: StdMutex::new(LedgerTradeIndex::default()),
            realized_pnl_cache: StdMutex::new(None),
            market_marks: StdRwLock::new(HashMap::new()),
            market_mark_pollers: StdMutex::new(HashSet::new()),
            vwap_runs: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn closed_trade_contributes_realized_pnl_fees_and_cash() {
        let store = store();
        let ledger = Ledger {
            initial_cash: 1_000.0,
            trades: serde_json::from_value(serde_json::json!([
                {"id":"b","symbol":"BTC-USDT","side":"buy","quantity":1.0,"execution_price":100.0,"timestamp":1,"fee":-1.0,"fee_known":true},
                {"id":"s","symbol":"BTC-USDT","side":"sell","quantity":1.0,"execution_price":120.0,"timestamp":2,"fee":-1.0,"fee_known":true}
            ])).unwrap(),
            ..Ledger::default()
        };
        let rates = BTreeMap::from([("USDT".to_owned(), 1.0)]);
        let account = account_snapshot_with_rates(&store, &ledger, &rates).unwrap();
        assert_eq!(account.cash, 1_018.0);
        assert_eq!(account.equity, 1_018.0);
        assert_eq!(account.realized_pnl, 18.0);
        assert_eq!(account.unrealized_pnl, 0.0);
        assert_eq!(account.fees_paid, 2.0);
        assert!(account.equity_available);
    }

    #[test]
    fn missing_quote_rate_marks_equity_unavailable() {
        let store = store();
        let ledger = Ledger {
            initial_cash: 1_000.0,
            trades: serde_json::from_value(serde_json::json!([
                {"id":"b","symbol":"BTC-XYZ","side":"buy","quantity":1.0,"execution_price":100.0,"timestamp":1}
            ])).unwrap(),
            ..Ledger::default()
        };
        let rates = BTreeMap::from([("USDT".to_owned(), 1.0)]);
        let account = account_snapshot_with_rates(&store, &ledger, &rates).unwrap();
        assert!(!account.equity_available);
        assert!(!account.fees_complete);
        assert_eq!(account.unknown_fee_fills, 1);
        let account_json = serde_json::to_value(&account).unwrap();
        assert_eq!(account_json["fees_complete"], false);
        assert_eq!(account_json["unknown_fee_fills"], 1);
    }

    #[test]
    fn legacy_unknown_fees_make_account_values_incomplete_but_known_zero_does_not() {
        let store = store();
        let rates = BTreeMap::from([("USDT".to_owned(), 1.0)]);
        let legacy = Ledger {
            initial_cash: 1_000.0,
            trades: serde_json::from_value(serde_json::json!([
                {"id":"legacy","symbol":"BTC-USDT","side":"buy","quantity":1.0,"execution_price":100.0,"timestamp":1}
            ])).unwrap(),
            ..Ledger::default()
        };
        let account = account_snapshot_with_rates(&store, &legacy, &rates).unwrap();
        assert!(account.equity_available);
        assert_eq!(account.cash, 900.0); // unknown fee omitted from numeric cash
        assert_eq!(account.fees_paid, 0.0);
        assert!(!account.fees_complete);
        assert_eq!(account.unknown_fee_fills, 1);

        let known_zero_store = store();
        let known_zero = Ledger {
            initial_cash: 1_000.0,
            trades: serde_json::from_value(serde_json::json!([
                {"id":"zero","symbol":"BTC-USDT","side":"buy","quantity":1.0,"execution_price":100.0,"timestamp":1,"fee":0.0,"fee_known":true}
            ])).unwrap(),
            ..Ledger::default()
        };
        let account = account_snapshot_with_rates(&known_zero_store, &known_zero, &rates).unwrap();
        assert!(account.equity_available);
        assert_eq!(account.fees_paid, 0.0);
        assert!(account.fees_complete);
        assert_eq!(account.unknown_fee_fills, 0);
    }
}
