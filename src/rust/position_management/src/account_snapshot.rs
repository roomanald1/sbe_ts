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
        + snapshot.cash_delta;
    let fees_paid = snapshot.fees_paid;
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
