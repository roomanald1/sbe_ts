struct PreparedMarketMatch {
    book: Arc<BookSnapshot>,
    matched: MatcherResponse,
    decision_timestamp: Option<u64>,
    decision_price: Option<f64>,
    decision_mid_price: Option<f64>,
    processing_timings_ms: BTreeMap<String, f64>,
}

async fn prepare_market_match(
    store: &Arc<PositionStore>,
    symbol: &str,
    request: &NewOrder,
    options: &MarketOrderOptions,
) -> Result<PreparedMarketMatch, (StatusCode, String)> {
    let mut timings = BTreeMap::new();
    store.ensure_subscription(symbol);
    let book = store.book(symbol).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Live OKX book unavailable".into(),
    ))?;
    let matching_started = Instant::now();
    reject_crossing_post_only(request, options, &book)?;
    let (decision_timestamp, decision_price, decision_mid_price) =
        execution_reference_prices(request, &book)?;
    let matched = match_book(&request.side, request.quantity, options.limit_price, &book);
    reject_unfillable_fok(request, options, &matched)?;
    timings.insert(
        "matching".into(),
        matching_started.elapsed().as_secs_f64() * 1_000.0,
    );
    Ok(PreparedMarketMatch {
        book,
        matched,
        decision_timestamp,
        decision_price,
        decision_mid_price,
        processing_timings_ms: timings,
    })
}

fn reject_crossing_post_only(
    request: &NewOrder,
    options: &MarketOrderOptions,
    book: &BookSnapshot,
) -> Result<(), (StatusCode, String)> {
    if !options.post_only {
        return Ok(());
    }
    let crosses = match (
        request.side.clone(),
        options.limit_price,
        book.bids.first(),
        book.asks.first(),
    ) {
        (TradeSide::Buy, Some(price), _, Some(ask)) => price >= ask.price,
        (TradeSide::Sell, Some(price), Some(bid), _) => price <= bid.price,
        _ => false,
    };
    if crosses {
        Err((
            StatusCode::CONFLICT,
            "Post-only limit would take liquidity; order was not placed".into(),
        ))
    } else {
        Ok(())
    }
}

fn execution_reference_prices(
    request: &NewOrder,
    book: &BookSnapshot,
) -> Result<(Option<u64>, Option<f64>, Option<f64>), (StatusCode, String)> {
    if !request.measure_execution_quality {
        return Ok((None, None, None));
    }
    let levels = match request.side {
        TradeSide::Buy => &book.asks,
        TradeSide::Sell => &book.bids,
    };
    let timestamp = Some(now_ms().map_err(internal_error)?);
    let price = levels
        .first()
        .map(|level| level.price)
        .filter(|price| price.is_finite() && *price > 0.0);
    let mid = book
        .bids
        .first()
        .zip(book.asks.first())
        .map(|(bid, ask)| (bid.price + ask.price) / 2.0)
        .filter(|price| price.is_finite() && *price > 0.0);
    Ok((timestamp, price, mid))
}

fn reject_unfillable_fok(
    request: &NewOrder,
    options: &MarketOrderOptions,
    matched: &MatcherResponse,
) -> Result<(), (StatusCode, String)> {
    if options.fill_or_kill && !request.reduce_only && matched.remaining_quantity > EPSILON {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "FOK order cancelled: only {} of {} was available",
                matched.filled_quantity, request.quantity
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod market_order_preparation_tests {
    use super::*;

    fn request(side: TradeSide, reduce_only: bool) -> NewOrder {
        NewOrder {
            symbol: "BTC-USDT".into(),
            side,
            order_type: OrderType::PostOnlyLimit,
            quantity: 1.0,
            limit_price: Some(100.0),
            measure_execution_quality: false,
            trigger_price: None,
            trigger_direction: None,
            trailing_amount: None,
            trailing_percent: None,
            expires_at: None,
            time_in_force: None,
            source: "manual".into(),
            client_order_id: None,
            reduce_only,
        }
    }

    fn book() -> BookSnapshot {
        BookSnapshot {
            bids: vec![BookLevel { price: 99.0, size: 2.0 }],
            asks: vec![BookLevel { price: 101.0, size: 2.0 }],
            updated_at_ms: 1,
        }
    }

    #[test]
    fn post_only_rejects_orders_that_cross_the_opposite_top_level() {
        let mut options = MarketOrderOptions {
            maker_fee_rate: 0.0,
            taker_fee_rate: 0.0,
            post_only: true,
            limit: true,
            fill_or_kill: false,
            limit_price: Some(101.0),
        };
        let error = reject_crossing_post_only(&request(TradeSide::Buy, false), &options, &book())
            .unwrap_err();
        assert_eq!(error.0, StatusCode::CONFLICT);

        options.limit_price = Some(99.0);
        let error = reject_crossing_post_only(&request(TradeSide::Sell, false), &options, &book())
            .unwrap_err();
        assert_eq!(error.0, StatusCode::CONFLICT);
    }

    #[test]
    fn post_only_allows_non_crossing_and_non_post_only_orders() {
        let mut options = MarketOrderOptions {
            maker_fee_rate: 0.0,
            taker_fee_rate: 0.0,
            post_only: true,
            limit: true,
            fill_or_kill: false,
            limit_price: Some(100.0),
        };
        assert!(reject_crossing_post_only(&request(TradeSide::Buy, false), &options, &book()).is_ok());
        options.post_only = false;
        options.limit_price = Some(101.0);
        assert!(reject_crossing_post_only(&request(TradeSide::Buy, false), &options, &book()).is_ok());
    }

    #[test]
    fn fok_rejects_partial_fill_but_reduce_only_may_partially_fill() {
        let req = request(TradeSide::Buy, false);
        let matched = MatcherResponse {
            reference_price: Some(101.0),
            fills: vec![],
            remaining_quantity: 0.25,
            filled_quantity: 0.75,
            average_execution_price: Some(101.0),
        };
        let options = MarketOrderOptions {
            maker_fee_rate: 0.0,
            taker_fee_rate: 0.0,
            post_only: false,
            limit: false,
            fill_or_kill: true,
            limit_price: None,
        };
        assert_eq!(reject_unfillable_fok(&req, &options, &matched).unwrap_err().0, StatusCode::CONFLICT);
        assert!(reject_unfillable_fok(&request(TradeSide::Buy, true), &options, &matched).is_ok());
        let complete = MatcherResponse { remaining_quantity: 0.0, ..matched };
        assert!(reject_unfillable_fok(&req, &options, &complete).is_ok());
    }
}
