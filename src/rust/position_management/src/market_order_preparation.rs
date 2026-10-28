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
    let book_wait_started = Instant::now();
    store.ensure_subscription(symbol);
    let book_was_missing = store.book(symbol).is_none();
    if book_was_missing {
        wait_for_market_book(store, symbol).await?;
    }
    let book = store.book(symbol).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Live OKX book unavailable".into(),
    ))?;
    timings.insert(
        if book_was_missing { "book_wait_cold" } else { "book_wait_active" }.into(),
        book_wait_started.elapsed().as_secs_f64() * 1_000.0,
    );

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

async fn wait_for_market_book(
    store: &PositionStore,
    symbol: &str,
) -> Result<(), (StatusCode, String)> {
    let wait = async {
        loop {
            let notified = store.book_changed.notified();
            if store.book(symbol).is_some() {
                break;
            }
            notified.await;
        }
    };
    timeout(Duration::from_secs(8), wait)
        .await
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Waiting for live OKX book timed out".into(),
            )
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
