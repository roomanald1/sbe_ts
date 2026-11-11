async fn run_hedge_evaluation(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let _ = maybe_place_portfolio_hedge(state).await?;
    Ok(())
}

async fn fresh_subscribed_market_book(
    symbol: &str,
) -> Option<serde_json::Value> {
    // The GET handler ensures the subscription itself, so a separate POST here
    // only adds a network round trip before the first book poll.
    for _ in 0..4 {
        if let Ok(market) = crate::service_rpc::position_get(&format!(
            "/internal/market-book?symbol={symbol}"
        )).await
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
