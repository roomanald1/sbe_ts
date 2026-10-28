async fn load_signal_market_snapshot(
    client: &reqwest::Client,
    position_url: &str,
    dashboard: &Dashboard,
    now: u64,
    signal: &Signal,
) -> Result<SignalMarketSnapshot, CycleResult> {
    let market_inputs_started = Instant::now();
    let cached_market = dashboard.markets.get(&signal.symbol).cloned();
    let cached_market_is_fresh = cached_market.as_ref().is_some_and(|market| {
        let updated_at = market
            .get("updated_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        updated_at > 0 && now.saturating_mul(1000).saturating_sub(updated_at) <= 5_000
    });
    let market = if cached_market_is_fresh {
        cached_market
    } else {
        fresh_subscribed_market_book(client, position_url, &signal.symbol).await
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
    let market_inputs_ms = market_inputs_started.elapsed().as_secs_f64() * 1000.0;
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
    let mid = (bid + ask) / 2.0;
    // Warm the paper manager's book subscription while the remaining order
    // checks run, so the submit path doesn't pay the feed's first-book delay.
    let warm_client = client.clone();
    let warm_url = format!(
        "{position_url}/internal/market-book/{}/subscribe",
        signal.symbol
    );
    tokio::spawn(async move {
        let _ = warm_client.post(warm_url).send().await;
    });
    Ok(SignalMarketSnapshot {
        market,
        bid,
        ask,
        mid,
        market_inputs_ms,
    })
}

struct FreshSignalCandidates<'a> {
    fresh_directional: Vec<&'a Signal>,
    neutral_scout_candidates: Vec<Signal>,
    stale_signal_count: usize,
    stalest_signal_age_seconds: u64,
    stale_symbols: Vec<String>,
    non_directional_count: usize,
    position_blocked_count: usize,
}

