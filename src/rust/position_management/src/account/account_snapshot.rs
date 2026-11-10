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
        AccountLedgerSnapshot {
            cash_delta, fees_paid, equity_available,
            positions: index.positions.get(&None).cloned().unwrap_or_default(),
        }
    });
    let cash = ledger.initial_cash
        + ledger.topups.iter().map(|topup| topup.amount).sum::<f64>()
        + snapshot.cash_delta
        + ledger
            .dated_carry_positions
            .iter()
            .map(|position| {
                if position.status.starts_with("closed") {
                    position.net_pnl_usdt.unwrap_or_default()
                } else {
                    position.quantity * (position.future_entry_price - position.spot_entry_price)
                        - position.entry_fees
                }
            })
            .sum::<f64>();
    let fees_paid = snapshot.fees_paid
        + ledger
            .dated_carry_positions
            .iter()
            .map(|position| position.entry_fees)
            .sum::<f64>();
    let mut equity_available = snapshot.equity_available;
    let positions = snapshot.positions.into_values().collect::<Vec<_>>();
    let books = store.books_snapshot();
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
        let Some(book) = books.get(&position.symbol) else {
            equity_available = false;
            continue;
        };
        let mark = if position.quantity > 0.0 {
            book.bids.first()
        } else {
            book.asks.first()
        };
        let Some(mark) = mark else {
            equity_available = false;
            continue;
        };
        let marked_value = position.quantity * mark.price * rate;
        equity += marked_value;
        let gross_unrealized = (mark.price - position.average_entry_price) * position.quantity;
        let close_fee = position.quantity.abs() * mark.price * position.estimated_exit_fee_rate;
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
    for position in ledger
        .dated_carry_positions
        .iter()
        .filter(|position| position.status.starts_with("closed"))
    {
        if let Some(pnl) = position.net_pnl_usdt.filter(|pnl| pnl.is_finite()) {
            realized_pnl += pnl * rates.get("USDT").copied().unwrap_or(0.0);
        } else {
            equity_available = false;
        }
    }
    Ok(AccountResponse {
        currency: "USD",
        cash,
        equity,
        realized_pnl,
        unrealized_pnl,
        fees_paid,
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
            books: StdRwLock::new(HashMap::new()),
            price_samples: Mutex::new(HashMap::new()),
            hedge_samples: Mutex::new(HashMap::new()),
            book_changed: Notify::new(),
            subscriptions: StdMutex::new(HashMap::new()),
            spot_symbols: Arc::new(Vec::new()),
            vwap_runs: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn closed_trade_contributes_realized_pnl_fees_and_cash() {
        let store = store();
        let ledger = Ledger {
            initial_cash: 1_000.0,
            trades: serde_json::from_value(serde_json::json!([
                {"id":"b","symbol":"BTC-USDT","side":"buy","quantity":1.0,"execution_price":100.0,"timestamp":1,"fee":-1.0},
                {"id":"s","symbol":"BTC-USDT","side":"sell","quantity":1.0,"execution_price":120.0,"timestamp":2,"fee":-1.0}
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
    }
}
