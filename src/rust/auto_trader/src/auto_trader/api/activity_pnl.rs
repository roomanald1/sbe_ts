async fn calculate_auto_unrealized_pnl(
    auto_positions: &[(String, TraderPosition)],
) -> Option<(f64, BTreeMap<String, f64>)> {
    let unrealized_pnl = if auto_positions.is_empty() {
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
            let Some(mark) = mark.filter(|price| price.is_finite() && *price > 0.0) else {
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
        complete.then_some((total, by_quote))
    };
    unrealized_pnl
}

