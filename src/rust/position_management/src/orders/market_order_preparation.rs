#[derive(Clone)]
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
    let requested_parent_order_id = request.client_order_id.as_deref();
    let execution_origin = if requested_parent_order_id.is_some_and(|id| id.starts_with("barrier-")) {
        "protective_barrier"
    } else if matches!(&request.order_type, OrderType::PostOnlyLimit) {
        "match_probe"
    } else {
        "market_match"
    };
    let barrier_request_id = (execution_origin == "protective_barrier").then(|| {
        format!("{}:barrier:{:016x}", requested_parent_order_id.unwrap(), request.quantity.to_bits())
    });
    let request_order_id = barrier_request_id.as_deref().or(requested_parent_order_id);
    let mut matched = paper_exchange_match(
        symbol,
        &request.side,
        request.quantity,
        options.limit_price,
        request_order_id,
        (execution_origin == "protective_barrier").then_some(requested_parent_order_id).flatten(),
        execution_origin,
        None,
    )
    .await?;
    matched.source_reported_filled_quantity = Some(matched.filled_quantity);
    for fill in &mut matched.fills {
        fill.source_quantity = Some(fill.quantity);
        fill.source_price = Some(fill.price);
    }
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
            fills.push(MatcherFill {
                quantity: filled,
                price: fill.price,
                source_execution_id: fill.source_execution_id,
                source_cursor: fill.source_cursor,
                source_cursor_kind: fill.source_cursor_kind,
                recorded_at_utc_ms: fill.recorded_at_utc_ms,
                fee: fill.fee,
                fee_currency: fill.fee_currency,
                fee_schedule_id: fill.fee_schedule_id,
                fee_schedule_version: fill.fee_schedule_version,
                fee_order_class: fill.fee_order_class,
                fee_schedule_evidence: fill.fee_schedule_evidence,
                source_price: fill.source_price.or(Some(fill.price)),
                source_quantity: fill.source_quantity.or(Some(fill.quantity)),
            });
            remaining -= filled;
        }
    }
    let filled_quantity = quantity.max(0.0) - remaining;
    result.fills = fills;
    result.filled_quantity = filled_quantity;
    result.remaining_quantity = remaining;
    result.average_execution_price =
        (filled_quantity > EPSILON).then_some(notional / filled_quantity);
}

fn source_match_identity_is_linked(
    result: &MatcherResponse,
    requested_order_id: Option<&str>,
    requested_quantity: f64,
) -> bool {
    let Some(order_id) = requested_order_id.filter(|value| !value.trim().is_empty()) else {
        return false;
    };
    // Protective barrier matching uses a deterministic per-attempt order ID
    // so a partial fill can be retried with its reduced quantity without
    // conflicting with the prior Paper Exchange request. The source response
    // keeps that attempt ID in `order_id` and the stable barrier ID in
    // `parent_order_id`; require the exact derivation and both identities.
    let barrier_attempt_id = order_id
        .starts_with("barrier-")
        .then(|| format!("{order_id}:barrier:{:016x}", requested_quantity.to_bits()));
    let order_identity_matches = result.order_id.as_deref() == Some(order_id)
        || barrier_attempt_id.as_deref().is_some_and(|attempt_id| {
            result.order_id.as_deref() == Some(attempt_id)
                && result.parent_order_id.as_deref() == Some(order_id)
        });
    result.schema_version.as_deref() == Some("paper_exchange_match:v1")
        && result.capture_complete == Some(true)
        && order_identity_matches
        && result
            .request_fingerprint
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        && result
            .source_epoch
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        && result
            .recorded_at_utc_ms
            .is_some_and(|timestamp| timestamp > 0)
        && result.source_cursor.is_some()
        && result.source_cursor_kind.as_deref() == Some("match_request")
}

fn source_match_is_complete(
    result: &MatcherResponse,
    requested_order_id: Option<&str>,
    requested_quantity: f64,
) -> bool {
    if !source_match_identity_is_linked(result, requested_order_id, requested_quantity)
        || result.source_reported_filled_quantity.is_none()
    {
        return false;
    }

    let Some(reported_filled_quantity) = result.source_reported_filled_quantity else {
        return false;
    };
    if !reported_filled_quantity.is_finite()
        || reported_filled_quantity < 0.0
        || !requested_quantity.is_finite()
        || reported_filled_quantity > requested_quantity + EPSILON
        || !result.filled_quantity.is_finite()
        || !result.remaining_quantity.is_finite()
    {
        return false;
    }

    let approximately_equal =
        |left: f64, right: f64| (left - right).abs() <= 1e-9 * left.abs().max(right.abs()).max(1.0);
    let mut source_ids = std::collections::HashSet::new();
    let mut source_cursors = std::collections::HashSet::new();
    let mut source_quantity_total = 0.0;
    let mut applied_quantity_total = 0.0;
    for fill in &result.fills {
        let Some(source_execution_id) = fill
            .source_execution_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return false;
        };
        if !source_ids.insert(source_execution_id)
            || fill
                .source_cursor
                .is_none_or(|cursor| !source_cursors.insert(cursor))
            || fill.source_cursor_kind.as_deref() != Some("match_fill")
            || fill
                .recorded_at_utc_ms
                .is_none_or(|timestamp| timestamp == 0)
            || !fill.quantity.is_finite()
            || fill.quantity <= 0.0
            || !fill.price.is_finite()
            || fill.price <= 0.0
        {
            return false;
        }
        let Some(source_quantity) = fill.source_quantity else {
            return false;
        };
        let Some(source_price) = fill.source_price else {
            return false;
        };
        if !source_quantity.is_finite()
            || source_quantity <= 0.0
            || !approximately_equal(source_quantity, fill.quantity)
            || !source_price.is_finite()
            || source_price <= 0.0
            || !approximately_equal(source_price, fill.price)
        {
            return false;
        }
        source_quantity_total += source_quantity;
        applied_quantity_total += fill.quantity;
    }

    approximately_equal(source_quantity_total, reported_filled_quantity)
        && approximately_equal(applied_quantity_total, result.filled_quantity)
        && approximately_equal(result.filled_quantity, reported_filled_quantity)
}

fn paper_exchange_fill_evidence(
    result: &MatcherResponse,
    expected_symbol: &str,
    requested_order_id: Option<&str>,
    requested_quantity: f64,
) -> Vec<PaperExchangeFillEvidence> {
    let identity_linked =
        source_match_identity_is_linked(result, requested_order_id, requested_quantity);
    let complete = source_match_is_complete(result, requested_order_id, requested_quantity);
    let mut execution_id_counts = std::collections::HashMap::new();
    let mut fill_cursor_counts = std::collections::HashMap::new();
    for execution_id in result
        .fills
        .iter()
        .filter_map(|fill| fill.source_execution_id.as_deref())
        .filter(|value| !value.trim().is_empty())
    {
        *execution_id_counts
            .entry(execution_id.to_owned())
            .or_insert(0usize) += 1;
    }
    for cursor in result.fills.iter().filter_map(|fill| fill.source_cursor) {
        *fill_cursor_counts.entry(cursor).or_insert(0usize) += 1;
    }

    result
        .fills
        .iter()
        .map(|fill| {
            let linked = identity_linked
                && fill
                    .source_execution_id
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .is_some_and(|value| execution_id_counts.get(value) == Some(&1))
                && fill
                    .source_cursor
                    .is_some_and(|cursor| fill_cursor_counts.get(&cursor) == Some(&1));
            let linked = linked
                && fill.source_cursor_kind.as_deref() == Some("match_fill")
                && fill
                    .recorded_at_utc_ms
                    .is_some_and(|timestamp| timestamp > 0);
            PaperExchangeFillEvidence {
                schema_version: linked.then(|| result.schema_version.clone()).flatten(),
                source_epoch: linked.then(|| result.source_epoch.clone()).flatten(),
                source_cursor: linked.then_some(fill.source_cursor).flatten(),
                request_fingerprint: linked.then(|| result.request_fingerprint.clone()).flatten(),
                execution_id: linked.then(|| fill.source_execution_id.clone()).flatten(),
                source_quantity: linked.then_some(fill.source_quantity).flatten(),
                source_price: linked.then_some(fill.source_price).flatten(),
                complete: Some(complete),
                fee: linked.then_some(fill.fee).flatten().filter(|fee| fee.is_finite()),
                fee_currency: linked
                    .then(|| fill.fee_currency.clone())
                    .flatten()
                    .filter(|currency| !currency.trim().is_empty()),
                fee_schedule_id: linked.then(|| fill.fee_schedule_id.clone()).flatten(),
                fee_schedule_version: linked
                    .then(|| fill.fee_schedule_version.clone())
                    .flatten(),
                fee_order_class: linked.then(|| fill.fee_order_class.clone()).flatten(),
                fee_schedule_evidence: linked
                    .then(|| fill.fee_schedule_evidence.clone())
                    .flatten()
                    .filter(|event| {
                        paper_exchange_fee_schedule_evidence_matches(
                            event,
                            fill,
                            expected_symbol,
                        )
                    }),
            }
        })
        .collect()
}

fn paper_exchange_fee_schedule_evidence_matches(
    event: &serde_json::Value,
    fill: &MatcherFill,
    expected_symbol: &str,
) -> bool {
    let quote_currency = expected_symbol
        .rsplit_once('-')
        .map(|(_, quote)| quote)
        .unwrap_or_default();
    let fee_currency = fill.fee_currency.as_deref().unwrap_or_default();
    let fee = fill.fee.unwrap_or(f64::NAN);
    let quantity = fill.quantity;
    let price = fill.price;
    let recorded_at = fill.recorded_at_utc_ms.unwrap_or_default();
    let schedule_id = fill.fee_schedule_id.as_deref().unwrap_or_default();
    let schedule_version = fill.fee_schedule_version.as_deref().unwrap_or_default();
    let order_class = fill.fee_order_class.as_deref().unwrap_or_default();
    let maker_rate = event["maker_rate"].as_f64().unwrap_or(f64::NAN);
    let taker_rate = event["taker_rate"].as_f64().unwrap_or(f64::NAN);
    let applied_rate = match order_class {
        "maker" => maker_rate,
        "taker" => taker_rate,
        _ => return false,
    };
    let effective_from = event["effective_from_utc_ms"].as_u64().unwrap_or(u64::MAX);
    let effective_to = event["effective_to_utc_ms"].as_u64();
    let expected_fee = -(quantity * price * applied_rate);
    event["event_type"] == "fee_schedule"
        && event["source_system"].as_str().is_some_and(|value| !value.trim().is_empty())
        && event["source_epoch"].as_str().is_some_and(|value| !value.trim().is_empty())
        && event["source_record_id"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
        && event["source_cursor"].as_u64().is_some_and(|cursor| cursor > 0)
        && event["source_timestamp_utc_ms"].as_u64().is_some_and(|timestamp| timestamp > 0)
        && event["symbol"] == expected_symbol
        && event["fee_schedule_id"] == schedule_id
        && event["fee_schedule_version"] == schedule_version
        && !schedule_id.trim().is_empty()
        && !schedule_version.trim().is_empty()
        && fee_currency == quote_currency
        && effective_from <= recorded_at
        && effective_to.is_none_or(|end| recorded_at < end)
        && maker_rate.is_finite()
        && (0.0..=0.1).contains(&maker_rate)
        && taker_rate.is_finite()
        && (0.0..=0.1).contains(&taker_rate)
        && fee.is_finite()
        && expected_fee.is_finite()
        && (fee - expected_fee).abs() <= 1e-9 * fee.abs().max(expected_fee.abs()).max(1.0)
}

async fn paper_exchange_match(
    symbol: &str,
    side: &TradeSide,
    quantity: f64,
    limit_price: Option<f64>,
    order_id: Option<&str>,
    parent_order_id: Option<&str>,
    execution_origin: &str,
    execution_price_override: Option<f64>,
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
            "order_id": order_id,
            "parent_order_id": parent_order_id,
            "execution_origin": execution_origin,
            "execution_price_override": execution_price_override,
        }),
    )
    .await
    .map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Paper exchange match failed: {error}"),
        )
    })?;
    serde_json::from_value(result).map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Invalid paper exchange result: {error}"),
        )
    })
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
        let error = reject_crossing_post_only(
            &request(TradeSide::Buy, false),
            &options,
            Some(99.0),
            Some(101.0),
        )
        .unwrap_err();
        assert_eq!(error.0, StatusCode::CONFLICT);

        options.limit_price = Some(99.0);
        let error = reject_crossing_post_only(
            &request(TradeSide::Sell, false),
            &options,
            Some(99.0),
            Some(101.0),
        )
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
        assert!(
            reject_crossing_post_only(
                &request(TradeSide::Buy, false),
                &options,
                Some(99.0),
                Some(101.0)
            )
            .is_ok()
        );
        options.post_only = false;
        options.limit_price = Some(101.0);
        assert!(
            reject_crossing_post_only(
                &request(TradeSide::Buy, false),
                &options,
                Some(99.0),
                Some(101.0)
            )
            .is_ok()
        );
    }
}
