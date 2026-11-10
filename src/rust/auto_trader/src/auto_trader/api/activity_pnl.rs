async fn calculate_auto_unrealized_pnl(
    auto_positions: &[(String, TraderPosition)],
    carry_positions: &[DatedCarryPosition],
) -> Option<(f64, BTreeMap<String, f64>)> {
    let unrealized_pnl = if auto_positions.is_empty()
        && !carry_positions.iter().any(|position| position.status == "open")
    {
        Some((0.0, BTreeMap::new()))
    } else {
        let base = std::env::var("POSITION_MANAGEMENT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3002".into());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_default();
        let mut total = 0.0;
        let mut by_quote = BTreeMap::<String, f64>::new();
        let mut complete = true;
        for (symbol, position) in auto_positions {
            let response = client
                .get(format!("{base}/internal/insights-inputs?symbol={symbol}"))
                .send()
                .await;
            let Ok(response) = response else {
                complete = false;
                break;
            };
            let Ok(inputs) = response.error_for_status() else {
                complete = false;
                break;
            };
            let Ok(inputs) = inputs.json::<serde_json::Value>().await else {
                complete = false;
                break;
            };
            let market = &inputs["markets"][&symbol];
            let mark = if position.quantity > 0.0 {
                market["bids"][0]["price"].as_f64()
            } else {
                market["asks"][0]["price"].as_f64()
            };
            let mark = mark.filter(|price| price.is_finite() && *price > 0.0)
                .or_else(|| {
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .ok()?
                        .as_millis() as u64;
                    let sample = market["price_samples"].as_array()?.last()?;
                    let timestamp = sample.get(0)?.as_u64()?;
                    let price = sample.get(1)?.as_f64()?;
                    (now_ms.saturating_sub(timestamp) <= 30_000
                        && price.is_finite()
                        && price > 0.0)
                        .then_some(price)
                });
            let Some(mark) = mark else {
                complete = false;
                break;
            };
            let pnl = (mark - position.average_entry_price) * position.quantity
                - position.entry_fees
                - position.quantity.abs() * mark * configured_taker_fee_rate();
            total += pnl;
            let quote = symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or("USD");
            *by_quote.entry(quote.to_owned()).or_default() += pnl;
        }
        for position in carry_positions.iter().filter(|position| position.status == "open") {
            let Some(pnl) = position
                .marked_net_pnl_usdt
                .filter(|pnl| pnl.is_finite())
            else {
                complete = false;
                break;
            };
            total += pnl;
            *by_quote.entry("USDT".to_owned()).or_default() += pnl;
        }
        complete.then_some((total, by_quote))
    };
    unrealized_pnl
}

