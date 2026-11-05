pub async fn run_usd_rate_stream(output: watch::Sender<Bytes>) {
    // USD index rates change slowly and the REST endpoint already returns the
    // complete currency set. Polling avoids keeping a second long-lived OKX
    // public WebSocket (which was repeatedly reset by the remote endpoint).
    const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
    let mut retry_delay = INITIAL_RECONNECT_DELAY;
    loop {
        match bootstrap_usd_rates().await {
            Ok(snapshot) => {
                publish_usd_rate_snapshot(&output, &snapshot);
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

fn publish_usd_rate_snapshot(output: &watch::Sender<Bytes>, snapshot: &UsdRateSnapshot) {
    if let Ok(bytes) = serde_json::to_vec(snapshot) {
        output.send_replace(Bytes::from(bytes));
    }
}

async fn bootstrap_usd_rates() -> Result<UsdRateSnapshot> {
    let response = reqwest::Client::new()
        .get("https://www.okx.com/api/v5/market/index-tickers?quoteCcy=USD")
        .send().await?.error_for_status()?.json::<IndexTickerRestResponse>().await?;
    parse_usd_rate_response(response)
}

fn parse_usd_rate_response(response: IndexTickerRestResponse) -> Result<UsdRateSnapshot> {
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

#[cfg(test)]
mod usd_rate_stream_tests {
    use super::{
        IndexTickerRestResponse, parse_usd_rate_response, publish_usd_rate_snapshot,
    };
    use serde_json::json;
    use bytes::Bytes;

    #[test]
    fn parses_only_positive_finite_usd_quotes_and_keeps_currency_timestamps() {
        let response: IndexTickerRestResponse = serde_json::from_value(json!({"data": [
            {"instId":"BTC-USD", "idxPx":"65000.5", "ts":"1700000000000"},
            {"instId":"ETH-USD", "idxPx":"3000", "ts":"1700000001000"},
            {"instId":"EUR-EUR", "idxPx":"1", "ts":"1700000002000"},
            {"instId":"BAD-USD", "idxPx":"NaN", "ts":"1"},
            {"instId":"OLD-USD", "idxPx":"2", "ts":"bad"},
            {"instId":"BROKEN", "idxPx":"1", "ts":"1"}
        ]})).unwrap();

        let snapshot = parse_usd_rate_response(response).unwrap();
        assert_eq!(snapshot.rates["USD"], 1.0);
        assert_eq!(snapshot.rates["BTC"], 65000.5);
        assert_eq!(snapshot.rates["ETH"], 3000.0);
        assert_eq!(snapshot.updated_at, 1_700_000_001_000);
        assert_eq!(snapshot.updated_at_by_currency["USD"], 0);
        assert_eq!(snapshot.updated_at_by_currency["BTC"], 1_700_000_000_000);
    }

    #[test]
    fn rejects_response_without_usable_usd_rates() {
        let response: IndexTickerRestResponse = serde_json::from_value(json!({"data": [
            {"instId":"BTC-USD", "idxPx":"0", "ts":"1"},
            {"instId":"ETH-EUR", "idxPx":"1", "ts":"1"}
        ]})).unwrap();
        assert!(parse_usd_rate_response(response).is_err());
    }

    #[test]
    fn publishes_serialized_rate_snapshots_to_watch_receivers() {
        let (sender, mut receiver) = tokio::sync::watch::channel(Bytes::new());
        let snapshot = parse_usd_rate_response(
            serde_json::from_value(json!({"data":[
                {"instId":"BTC-USD","idxPx":"10","ts":"20"}
            ]}))
            .unwrap(),
        )
        .unwrap();
        publish_usd_rate_snapshot(&sender, &snapshot);
        let serialized: serde_json::Value = serde_json::from_slice(&receiver.borrow()).unwrap();
        assert_eq!(serialized["rates"]["BTC"], 10.0);
    }
}
