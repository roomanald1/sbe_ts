async fn ensure_paper_capacity(
    store: &PositionStore,
    ledger: &Ledger,
    symbol: &str,
    side: &TradeSide,
    quantity: f64,
    price: f64,
    fee_rate: f64,
) -> Result<(), (StatusCode, String)> {
    let rates = cached_usd_rates_for_order().await?.rates;
    ensure_paper_capacity_with_rates(
        store, ledger, &rates, symbol, side, quantity, price, fee_rate, None,
    )
    .await
}
async fn ensure_paper_capacity_with_rates(
    store: &PositionStore,
    ledger: &Ledger,
    rates: &BTreeMap<String, f64>,
    symbol: &str,
    side: &TradeSide,
    quantity: f64,
    price: f64,
    fee_rate: f64,
    mut timings: Option<&mut BTreeMap<String, f64>>,
) -> Result<(), (StatusCode, String)> {
    let quote = symbol
        .split_once('-')
        .map(|(_, quote)| quote)
        .unwrap_or_default();
    let quote_rate = *rates.get(quote).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        format!("No OKX USD conversion rate for {quote}"),
    ))?;
    let account_started = Instant::now();
    let account = account_snapshot_with_rates(store, ledger, rates)?;
    if let Some(timings) = timings.as_deref_mut() {
        timings.insert(
            "capacity_account_snapshot".to_owned(),
            account_started.elapsed().as_secs_f64() * 1_000.0,
        );
    }
    if !account.equity_available {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "Account equity cannot be valued until every open position has a live mark and USD conversion rate".into()));
    }
    let reservation_scan_started = Instant::now();
    let mut reserved_buying_power = 0.0;
    for barrier in ledger
        .barrier_orders
        .iter()
        .filter(|order| order.status == "armed" && matches!(order.side, TradeSide::Buy))
    {
        let barrier_quote = barrier
            .symbol
            .split_once('-')
            .map(|(_, quote)| quote)
            .unwrap_or_default();
        let rate = *rates.get(barrier_quote).ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("No OKX USD conversion rate for {barrier_quote}"),
        ))?;
        reserved_buying_power += barrier.remaining_quantity
            * barrier.trigger_price
            * rate
            * (1.0 - configured_fee_rates().taker);
    }
    for order in &ledger.orders {
        if matches!(order.side, TradeSide::Buy) {
            let order_quote = order
                .symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or_default();
            let rate = *rates.get(order_quote).ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                format!("No OKX USD conversion rate for {order_quote}"),
            ))?;
            reserved_buying_power +=
                (order.remaining_quantity * order.limit_price + order.estimated_fee) * rate;
        }
    }
    let runs = store.vwap_runs.try_lock().map_err(|_| {
        (StatusCode::SERVICE_UNAVAILABLE, "VWAP reservations busy".to_owned())
    })?;
    let books = store.books_snapshot();
    let mut reserved_sell = ledger
        .barrier_orders
        .iter()
        .filter(|order| {
            order.status == "armed"
                && order.symbol == symbol
                && matches!(order.side, TradeSide::Sell)
        })
        .map(|order| order.remaining_quantity)
        .sum::<f64>();
    for entry in runs.values() {
        let status = entry.status.try_lock().map_err(|_| {
            (StatusCode::SERVICE_UNAVAILABLE, "VWAP reservation busy".to_owned())
        })?;
        if status.status != "running" {
            continue;
        }
        let remaining = (status.quantity - status.executed_quantity).max(0.0);
        if matches!(status.side, TradeSide::Buy) {
            let run_quote = status
                .symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or_default();
            let rate = *rates.get(run_quote).ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                format!("No OKX USD conversion rate for {run_quote}"),
            ))?;
            let run_price = books
                .get(&status.symbol)
                .and_then(|book| book.asks.first())
                .map(|level| level.price)
                .ok_or((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Cannot reserve VWAP without a live ask price".into(),
                ))?;
            reserved_buying_power +=
                remaining * run_price * rate * (1.0 - configured_fee_rates().taker);
        } else if status.symbol == symbol {
            reserved_sell += remaining;
        }
    }
    if let Some(timings) = timings.as_deref_mut() {
        timings.insert(
            "capacity_reservation_scan".to_owned(),
            reservation_scan_started.elapsed().as_secs_f64() * 1_000.0,
        );
    }
    if matches!(side, TradeSide::Buy) {
        let required = quantity * price * quote_rate * (1.0 - fee_rate);
        let available = account.cash - reserved_buying_power;
        if required > available + EPSILON {
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "Insufficient paper account buying power: need {:.2} USD, have {:.2} USD",
                    required,
                    available.max(0.0)
                ),
            ));
        }
    } else {
        let positions = summarize_positions(&ledger.trades).positions;
        let held = positions
            .iter()
            .find(|position| position.symbol == symbol)
            .map(|position| position.quantity.max(0.0))
            .unwrap_or(0.0);
        let reserved = ledger
            .orders
            .iter()
            .filter(|order| order.symbol == symbol && matches!(order.side, TradeSide::Sell))
            .map(|order| order.remaining_quantity)
            .sum::<f64>()
            + reserved_sell;
        let new_borrow = (quantity - (held - reserved).max(0.0)).max(0.0);
        let short_exposure = positions
            .iter()
            .filter(|position| position.quantity < -EPSILON)
            .map(|position| {
                let mark = books
                    .get(&position.symbol)
                    .and_then(|book| book.asks.first())
                    .map(|level| level.price)
                    .unwrap_or(position.average_entry_price);
                let quote = position
                    .symbol
                    .split_once('-')
                    .map(|(_, quote)| quote)
                    .unwrap_or_default();
                position.quantity.abs() * mark * rates.get(quote).copied().unwrap_or(0.0)
            })
            .sum::<f64>();
        let pending_short = ledger
            .orders
            .iter()
            .enumerate()
            .filter(|(index, order)| {
                matches!(order.side, TradeSide::Sell)
                    && !ledger.orders[..*index].iter().any(|prior| {
                        prior.symbol == order.symbol && matches!(prior.side, TradeSide::Sell)
                    })
            })
            .map(|(_, order)| {
                let long = positions
                    .iter()
                    .find(|position| position.symbol == order.symbol)
                    .map(|position| position.quantity.max(0.0))
                    .unwrap_or(0.0);
                let pending = ledger
                    .orders
                    .iter()
                    .filter(|other| {
                        other.symbol == order.symbol && matches!(other.side, TradeSide::Sell)
                    })
                    .map(|other| other.remaining_quantity)
                    .sum::<f64>();
                let quote = order
                    .symbol
                    .split_once('-')
                    .map(|(_, quote)| quote)
                    .unwrap_or_default();
                (pending - long).max(0.0)
                    * order.limit_price
                    * rates.get(quote).copied().unwrap_or(0.0)
            })
            .sum::<f64>();
        let exposure = short_exposure + pending_short + new_borrow * price * quote_rate;
        let limit = account.equity.max(0.0) * account.max_leverage;
        if exposure > limit + EPSILON {
            return Err((StatusCode::CONFLICT, format!("Paper spot margin limit exceeded: short exposure {:.2} USD exceeds {:.2} USD ({:.1}x equity)", exposure, limit, account.max_leverage)));
        }
    }
    Ok(())
}
