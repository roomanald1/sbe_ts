struct PreparedMarketMatch {
    matched: MatcherResponse,
    decision_timestamp: Option<u64>,
    decision_price: Option<f64>,
    decision_mid_price: Option<f64>,
    processing_timings_ms: BTreeMap<String, f64>,
}

async fn prepare_market_match(
    symbol: &str,
    request: &NewOrder,
    options: &MarketOrderOptions,
) -> Result<PreparedMarketMatch, (StatusCode, String)> {
    let mut timings = BTreeMap::new();
    let matching_started = Instant::now();
    let matched = paper_exchange_match(symbol, &request.side, request.quantity, options.limit_price).await?;
    reject_crossing_post_only(request, options, matched.best_bid, matched.best_ask)?;
    let (decision_timestamp, decision_price, decision_mid_price) = execution_reference_prices(
        request,
        matched.book_updated_at_ms,
        matched.reference_price,
        matched.best_bid,
        matched.best_ask,
    );
    timings.insert(
        "matching".into(),
        matching_started.elapsed().as_secs_f64() * 1_000.0,
    );
    Ok(PreparedMarketMatch {
        matched,
        decision_timestamp,
        decision_price,
        decision_mid_price,
        processing_timings_ms: timings,
    })
}

fn limit_match_quantity(result: &mut MatcherResponse, quantity: f64) {
    let mut remaining = quantity.max(0.0);
    let mut notional = 0.0;
    let mut fills = Vec::with_capacity(result.fills.len());
    for fill in result.fills.drain(..) {
        let filled = remaining.min(fill.quantity);
        if filled > EPSILON {
            notional += filled * fill.price;
            fills.push(MatcherFill { quantity: filled, price: fill.price });
            remaining -= filled;
        }
    }
    let filled_quantity = quantity.max(0.0) - remaining;
    result.fills = fills;
    result.filled_quantity = filled_quantity;
    result.remaining_quantity = remaining;
    result.average_execution_price = (filled_quantity > EPSILON)
        .then_some(notional / filled_quantity);
}

async fn paper_exchange_match(
    symbol: &str,
    side: &TradeSide,
    quantity: f64,
    limit_price: Option<f64>,
) -> Result<MatcherResponse, (StatusCode, String)> {
    let side = match side {
        TradeSide::Buy => "buy",
        TradeSide::Sell => "sell",
    };
    let result = paper_exchange_client::paper_exchange_post(
        "/internal/orders/match",
        serde_json::json!({
            "symbol": symbol,
            "side": side,
            "quantity": quantity,
            "limit_price": limit_price,
        }),
    )
    .await
    .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Paper exchange match failed: {error}")))?;
    serde_json::from_value(result)
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Invalid paper exchange result: {error}")))
}

fn reject_crossing_post_only(
    request: &NewOrder,
    options: &MarketOrderOptions,
    best_bid: Option<f64>,
    best_ask: Option<f64>,
) -> Result<(), (StatusCode, String)> {
    if !options.post_only {
        return Ok(());
    }
    let crosses = match (
        request.side.clone(),
        options.limit_price,
        best_bid,
        best_ask,
    ) {
        (TradeSide::Buy, Some(price), _, Some(ask)) => price >= ask,
        (TradeSide::Sell, Some(price), Some(bid), _) => price <= bid,
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
    updated_at_ms: u64,
    reference_price: Option<f64>,
    best_bid: Option<f64>,
    best_ask: Option<f64>,
) -> (Option<u64>, Option<f64>, Option<f64>) {
    if !request.measure_execution_quality {
        return (None, None, None);
    }
    let timestamp = Some(updated_at_ms);
    let price = reference_price.filter(|price| price.is_finite() && *price > 0.0);
    let mid = best_bid
        .zip(best_ask)
        .map(|(bid, ask)| (bid + ask) / 2.0)
        .filter(|price| price.is_finite() && *price > 0.0);
    (timestamp, price, mid)
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
            lifecycle: None,
            reduce_only,
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
        let error = reject_crossing_post_only(&request(TradeSide::Buy, false), &options, Some(99.0), Some(101.0))
            .unwrap_err();
        assert_eq!(error.0, StatusCode::CONFLICT);

        options.limit_price = Some(99.0);
        let error = reject_crossing_post_only(&request(TradeSide::Sell, false), &options, Some(99.0), Some(101.0))
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
        assert!(reject_crossing_post_only(&request(TradeSide::Buy, false), &options, Some(99.0), Some(101.0)).is_ok());
        options.post_only = false;
        options.limit_price = Some(101.0);
        assert!(reject_crossing_post_only(&request(TradeSide::Buy, false), &options, Some(99.0), Some(101.0)).is_ok());
    }

}
