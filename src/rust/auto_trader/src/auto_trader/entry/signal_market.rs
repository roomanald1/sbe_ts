async fn load_signal_market_snapshot(
    dashboard: &Dashboard,
    now: u64,
    signal: &Signal,
) -> Result<SignalMarketSnapshot, CycleResult> {
    let cache_lookup_started = Instant::now();
    let cached_market = dashboard.markets.get(&signal.symbol).cloned();
    let market_cache_lookup_ms = cache_lookup_started.elapsed().as_secs_f64() * 1_000.0;
    let cached_market_is_fresh = cached_market.as_ref().is_some_and(|market| {
        let updated_at = market
            .get("updated_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        updated_at > 0 && now.saturating_mul(1000).saturating_sub(updated_at) <= 5_000
    });
    let (market, market_http_fetch_ms) = if cached_market_is_fresh {
        (cached_market, 0.0)
    } else {
        let fetch_started = Instant::now();
        let market = fresh_subscribed_market_book(&signal.symbol).await;
        (market, fetch_started.elapsed().as_secs_f64() * 1_000.0)
    };
    let Some(market) = market else {
        return Err(skipped_for_signal(
            signal,
            &format!(
                "Waiting for a fresh subscribed local order book for {}",
                signal.symbol
            ),
        ));
    };
    let bid = market["bids"][0]["price"].as_f64();
    let ask = market["asks"][0]["price"].as_f64();
    let Some((bid, ask)) = bid
        .zip(ask)
        .filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid)
    else {
        return Err(skipped_for_signal(
            signal,
            &format!(
                "No current order book for {}; subscribed and waiting for a fresh book",
                signal.symbol
            ),
        ));
    };
    let mid = bid + (ask - bid) * 0.5;
    Ok(SignalMarketSnapshot {
        market,
        bid,
        ask,
        mid,
        market_cache_lookup_ms,
        market_http_fetch_ms,
    })
}

struct FreshSignalCandidates<'a> {
    fresh_directional: Vec<&'a Signal>,
    stale_signal_count: usize,
    stalest_signal_age_seconds: u64,
    stale_symbols: Vec<String>,
    non_directional_count: usize,
    position_blocked_count: usize,
    position_block_details: Vec<String>,
}
