async fn run_hedge_evaluation(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = state.client.clone();
    let _ = maybe_place_portfolio_hedge(&client, &position_url, state).await?;
    Ok(())
}

async fn fresh_subscribed_market_book(
    client: &reqwest::Client,
    position_url: &str,
    symbol: &str,
) -> Option<serde_json::Value> {
    // The GET handler ensures the subscription itself, so a separate POST here
    // only adds a network round trip before the first book poll.
    let market_url = format!("{position_url}/internal/market-book?symbol={symbol}");
    for _ in 0..4 {
        if let Ok(response) = client
            .get(&market_url)
            .timeout(Duration::from_millis(500))
            .send()
            .await
            && let Ok(response) = response.error_for_status()
            && let Ok(market) = response.json::<serde_json::Value>().await
        {
            let updated_at = market
                .get("updated_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            if updated_at > 0 && now_ms.saturating_sub(updated_at) <= 5_000 {
                return Some(market);
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    None
}
