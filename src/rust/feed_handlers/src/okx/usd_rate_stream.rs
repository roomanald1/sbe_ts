pub async fn run_usd_rate_stream(output: watch::Sender<Bytes>) {
    // USD index rates change slowly and the REST endpoint already returns the
    // complete currency set. Polling avoids keeping a second long-lived OKX
    // public WebSocket (which was repeatedly reset by the remote endpoint).
    const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
    let mut retry_delay = INITIAL_RECONNECT_DELAY;
    loop {
        match bootstrap_usd_rates().await {
            Ok(snapshot) => {
                if let Ok(bytes) = serde_json::to_vec(&snapshot) {
                    output.send_replace(Bytes::from(bytes));
                }
                retry_delay = INITIAL_RECONNECT_DELAY;
                sleep(REFRESH_INTERVAL).await;
            }
            Err(error) => {
                crate::service_log!("ERROR", "Could not refresh OKX USD index rates: {error:#}");
                sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(MAX_RECONNECT_DELAY);
            }
        }
    }
}

async fn bootstrap_usd_rates() -> Result<UsdRateSnapshot> {
    let response = reqwest::Client::new()
        .get("https://www.okx.com/api/v5/market/index-tickers?quoteCcy=USD")
        .send().await?.error_for_status()?.json::<IndexTickerRestResponse>().await?;
    let mut rates = BTreeMap::from([("USD".to_owned(), 1.0)]);
    let mut updated_at_by_currency = BTreeMap::from([("USD".to_owned(), 0)]);
    let mut updated_at = 0;
    for item in response.data {
        let Some((currency, quote)) = item.instrument.split_once('-') else { continue; };
        let Ok(rate) = item.index_price.parse::<f64>() else { continue; };
        let Ok(timestamp) = item.ts.parse::<u64>() else { continue; };
        if quote != "USD" || !rate.is_finite() || rate <= 0.0 { continue; }
        let currency = currency.to_owned();
        rates.insert(currency.clone(), rate);
        updated_at_by_currency.insert(currency, timestamp);
        updated_at = updated_at.max(timestamp);
    }
    if rates.len() == 1 { anyhow::bail!("OKX returned no usable USD index rates"); }
    Ok(UsdRateSnapshot { rates, updated_at, updated_at_by_currency })
}
