async fn calculate_auto_unrealized_pnl(
    auto_positions: &[(String, TraderPosition)],
    carry_positions: &[DatedCarryPosition],
) -> Option<(f64, BTreeMap<String, f64>)> {
    if auto_positions.is_empty()
        && !carry_positions.iter().any(|position| position.status == "open")
    {
        return Some((0.0, BTreeMap::new()));
    }

    let mut input_tasks = JoinSet::new();
    for (symbol, position) in auto_positions {
        let symbol = symbol.clone();
        let quantity = position.quantity;
        let average_entry_price = position.average_entry_price;
        let entry_fees = position.entry_fees;
        input_tasks.spawn(async move {
            let inputs = crate::service_rpc::execution_get(&format!(
                "/internal/insights-inputs?symbol={symbol}"
            ))
            .await;
            (symbol, quantity, average_entry_price, entry_fees, inputs)
        });
    }

    let mut total = 0.0;
    let mut by_quote = BTreeMap::<String, f64>::new();
    let mut complete = true;
    while let Some(result) = input_tasks.join_next().await {
        let Ok((symbol, quantity, average_entry_price, entry_fees, inputs)) = result else {
            complete = false;
            continue;
        };
        let Ok(inputs) = inputs else {
            complete = false;
            continue;
        };
        let market = &inputs["markets"][&symbol];
        let mark = if quantity > 0.0 {
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
            continue;
        };
        let pnl = (mark - average_entry_price) * quantity
            - entry_fees
            - quantity.abs() * mark * configured_taker_fee_rate();
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
            continue;
        };
        total += pnl;
        *by_quote.entry("USDT".to_owned()).or_default() += pnl;
    }
    complete.then_some((total, by_quote))
}

